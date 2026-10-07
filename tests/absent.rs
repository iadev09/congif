#![cfg(feature = "json")]

use std::path::PathBuf;

use congif::{
    ConfigFile, ConfigReader, Configurable, Error, Result, async_trait,
};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Listeners {
    #[serde(default)]
    names: Vec<String>,
}

/// An owner whose source may legitimately not exist: no file, no listeners.
struct Optional(PathBuf);

#[async_trait]
impl Configurable for Optional {
    type Config = Listeners;

    fn source(&self) -> ConfigFile {
        ConfigFile::json(&self.0)
    }

    fn absent_source(&self) -> Option<Listeners> {
        Some(Listeners::default())
    }

    async fn configure(
        &self,
        _: &Listeners,
    ) -> Result<()> {
        Ok(())
    }
}

/// An owner that keeps the default: a missing file is an error.
struct Required(PathBuf);

#[async_trait]
impl Configurable for Required {
    type Config = Listeners;

    fn source(&self) -> ConfigFile {
        ConfigFile::json(&self.0)
    }

    async fn configure(
        &self,
        _: &Listeners,
    ) -> Result<()> {
        Ok(())
    }
}

fn missing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/absent/does-not-exist.json")
}

#[tokio::test]
async fn an_absent_source_is_the_owner_s_declared_value_or_a_read_error() {
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();

    let optional = Optional(missing());
    assert_eq!(reader.prepare(&optional).await.unwrap(), Listeners::default());

    let required = Required(missing());
    match reader.prepare(&required).await {
        Err(Error::Read { path, source }) => {
            assert_eq!(path, missing());
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!(
            "a missing required source must stay a read error: {other:?}"
        ),
    }
}

#[tokio::test]
async fn a_batch_prepares_an_absent_optional_source_without_failing() {
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    let optional = Optional(missing());

    reader.prepare_owners(&[&optional]).await.unwrap();

    assert_eq!(*reader.snapshot::<Optional>().unwrap(), Listeners::default());
}
