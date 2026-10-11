//! A fake executable adapter checks the reader contract without a language VM.

use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use congif::{
    BoxError, ConfigFile, ConfigParser, ConfigReader, ConfigSource,
    ConfigValue, Configurable, Format, Result, Variables, async_trait,
};
use dotenw::{Environment, MergeMode};
use serde::Deserialize;

struct ExecutableParser(Arc<AtomicU64>);

#[async_trait]
impl ConfigParser for ExecutableParser {
    fn extensions(&self) -> &[&str] {
        &["executable"]
    }
    fn expand_variables(
        &self,
        text: &str,
        _: &Variables,
    ) -> Result<String> {
        Ok(text.to_owned())
    }
    async fn parse(
        &self,
        _: ConfigSource<'_>,
    ) -> std::result::Result<ConfigValue, BoxError> {
        Err("environment was not delivered to the adapter".into())
    }
    async fn parse_with_variables(
        &self,
        source: ConfigSource<'_>,
        variables: Option<&Variables>,
    ) -> std::result::Result<ConfigValue, BoxError> {
        assert_eq!(
            std::str::from_utf8(source.bytes)?.trim(),
            "return env('VALUE');"
        );
        self.0.fetch_add(1, Ordering::Relaxed);
        let value = variables
            .and_then(|values| values.get("VALUE"))
            .ok_or("VALUE missing")?;
        Ok(ConfigValue::String(value.clone()))
    }
}

#[derive(Deserialize, PartialEq)]
struct Settings(String);

struct Owner<const N: u8> {
    path: PathBuf,
}

#[async_trait]
impl<const N: u8> Configurable for Owner<N> {
    type Config = Settings;
    fn source(&self) -> ConfigFile {
        ConfigFile::new(&self.path, Format::Custom("executable".into()))
    }
    async fn configure(
        &self,
        _: &Settings,
    ) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn executable_parser_receives_variables_without_rewriting_source() {
    let reader = Arc::new(ConfigReader::new());
    reader.register(ExecutableParser(Arc::new(AtomicU64::new(0)))).unwrap();
    let variables = Variables::from([("VALUE".into(), "first".into())]);
    let settings: Settings = reader
        .parse_with_variables(
            ConfigSource {
                path: std::path::Path::new("settings.executable"),
                bytes: b"return env('VALUE');",
            },
            &variables,
        )
        .await
        .unwrap();
    assert_eq!(settings.0, "first");
}

fn bind<const N: u8>(
    reader: &ConfigReader,
    value: &str,
) {
    let handle = reader
        .environments()
        .register(Environment::new().with_values(
            Variables::from([("VALUE".into(), value.into())]),
            MergeMode::Override,
        ))
        .unwrap();
    reader.bind_environment::<Owner<N>>(handle).unwrap();
}

#[tokio::test]
async fn executable_parser_batch_separates_environments_and_reuses_equal_values()
 {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/executable.config");
    let parses = Arc::new(AtomicU64::new(0));
    let reader = ConfigReader::new();
    reader.register(ExecutableParser(parses.clone())).unwrap();
    bind::<1>(&reader, "first");
    bind::<2>(&reader, "second");
    bind::<3>(&reader, "first");
    let first = Owner::<1> { path: path.clone() };
    let second = Owner::<2> { path: path.clone() };
    let same = Owner::<3> { path };
    reader.prepare_owners(&[&first, &second, &same]).await.unwrap();
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().0, "first");
    assert_eq!(reader.snapshot::<Owner<2>>().unwrap().0, "second");
    assert_eq!(reader.snapshot::<Owner<3>>().unwrap().0, "first");
    assert_eq!(parses.load(Ordering::Relaxed), 2);
}
