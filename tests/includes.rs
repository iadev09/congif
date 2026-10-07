#![cfg(all(feature = "yaml", feature = "toml"))]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use congif::{
    ConfigFile, ConfigParser, ConfigReader, ConfigValue, Configurable, Result,
    async_trait,
};
use serde::Deserialize;

#[derive(Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Settings {
    value: u64,
}

struct Owner<const N: u8> {
    path: PathBuf,
    reject: AtomicBool,
}

#[async_trait]
impl<const N: u8> Configurable for Owner<N> {
    type Config = Settings;

    fn source(&self) -> ConfigFile {
        ConfigFile::yaml(self.path.clone())
    }

    async fn configure(
        &self,
        _: &Settings,
    ) -> Result<()> {
        Ok(())
    }

    fn validate_config(
        &self,
        _: &Settings,
    ) -> Result<()> {
        if self.reject.load(Ordering::Relaxed) {
            return Err(congif::Error::invalid("rejected candidate"));
        }
        Ok(())
    }
}

/// An owner whose document is completed by its includes: the sum of every
/// part's `value`, merged in `read_config`.
struct Merging(PathBuf);

#[async_trait]
impl Configurable for Merging {
    type Config = Settings;

    fn source(&self) -> ConfigFile {
        ConfigFile::yaml(self.0.clone())
    }

    async fn read_config(
        &self,
        reader: &ConfigReader,
    ) -> Result<Settings> {
        let mut total = reader.read_source(self).await?;
        for include in reader.includes_for(&self.0)? {
            let part: Settings =
                reader.read_include(self, &ConfigFile::yaml(include)).await?;
            total.value += part.value;
        }
        Ok(total)
    }

    async fn configure(
        &self,
        _: &Settings,
    ) -> Result<()> {
        Ok(())
    }
}

struct CountingYaml(Arc<AtomicU64>);

#[async_trait]
impl ConfigParser for CountingYaml {
    fn extensions(&self) -> &[&str] {
        &["yaml", "yml"]
    }

    fn includes(
        &self,
        value: &mut ConfigValue,
    ) -> std::result::Result<Vec<PathBuf>, congif::BoxError> {
        congif::parsers::YamlParser.includes(value)
    }

    async fn parse(
        &self,
        source: congif::ConfigSource<'_>,
    ) -> std::result::Result<ConfigValue, congif::BoxError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        congif::parsers::YamlParser.parse(source).await
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "congif-includes-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let fixture = Self(directory);
        fixture.replace("initial.yaml");
        fixture
    }

    fn path(&self) -> PathBuf {
        self.0.join("settings.yaml")
    }

    fn replace(
        &self,
        name: &str,
    ) {
        std::fs::copy(source(name), self.path()).unwrap();
    }

    fn owner<const N: u8>(&self) -> Owner<N> {
        Owner { path: self.path(), reject: AtomicBool::new(false) }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn source(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/includes")
        .join(name)
}

fn reader() -> ConfigReader {
    ConfigReader::new().with_builtin_parsers().unwrap()
}

#[tokio::test]
async fn yaml_and_toml_discover_nested_path_declarations_without_merging() {
    let reader = reader();
    for name in ["nested.yaml", "nested.toml"] {
        let path = source(name);
        let parsed: ConfigValue = reader.read(&path).await.unwrap();
        let ConfigValue::Map(map) = parsed else { panic!("fixture is a map") };
        assert!(!map.contains_key(&ConfigValue::String("includes".into())));
        assert_eq!(
            reader.includes_for(&path).unwrap(),
            vec![source("first.yaml"), source("second.yaml")]
        );
    }
    // Includes are dependencies, not a request to read or merge their contents.
    assert!(!source("first.yaml").exists());
}

#[tokio::test]
async fn invalid_path_values_are_errors_even_when_the_schema_ignores_them() {
    let reader = reader();
    for name in [
        "number.yaml",
        "boolean.yaml",
        "object.yaml",
        "null.yaml",
        "mixed.yaml",
        "empty.yaml",
        "number.toml",
    ] {
        let error = reader
            .read::<Settings>(source(name))
            .await
            .err()
            .expect("invalid include must fail");
        assert!(
            matches!(error, congif::Error::Parse { .. }),
            "{name}: {error}"
        );
        assert!(reader.includes_for(source(name)).unwrap().is_empty());
    }
}

#[tokio::test]
async fn dependency_index_tracks_the_latest_read_even_when_typed_validation_fails()
 {
    let fixture = Fixture::new();
    let first = fixture.owner::<1>();
    let second = fixture.owner::<2>();
    let reader = reader();
    reader.initialize_owners(&[&first, &second]).await.unwrap();
    for owner in [&first as &dyn congif::ConfigRegistration, &second] {
        reader.configure_registered(owner).await.unwrap();
        assert!(
            reader
                .sources_for(owner)
                .unwrap()
                .contains(&fixture.0.join("first.yaml"))
        );
    }
    fixture.replace("replacement.yaml");
    assert!(
        reader
            .prepare_changed_owners(&[&first, &second])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        vec![fixture.0.join("second.yaml")]
    );
    fixture.replace("initial.yaml");
    let active = reader.snapshot::<Owner<1>>().unwrap();
    second.reject.store(true, Ordering::Relaxed);
    assert!(reader.prepare_owners(&[&first, &second]).await.is_err());
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        vec![fixture.0.join("first.yaml")]
    );
    assert!(Arc::ptr_eq(&active, &reader.snapshot::<Owner<1>>().unwrap()));
    fixture.replace("without.yaml");
    assert!(reader.prepare_owners(&[&first, &second]).await.is_err());
    assert!(reader.includes_for(fixture.path()).unwrap().is_empty());
    assert!(Arc::ptr_eq(&active, &reader.snapshot::<Owner<1>>().unwrap()));
    second.reject.store(false, Ordering::Relaxed);
    fixture.replace("without.yaml");
    reader.prepare_owners(&[&first, &second]).await.unwrap();
    assert!(reader.includes_for(fixture.path()).unwrap().is_empty());
}

#[tokio::test]
async fn rejected_schema_keeps_new_known_paths_for_retry_without_publishing_config()
 {
    let fixture = Fixture::new();
    let owner = fixture.owner::<1>();
    let reader = reader();
    reader.initialize_owners(&[&owner]).await.unwrap();
    reader.configure_registered(&owner).await.unwrap();
    let active = reader.snapshot::<Owner<1>>().unwrap();
    fixture.replace("invalid_schema.yaml");
    assert!(reader.prepare_owners(&[&owner]).await.is_err());
    assert!(Arc::ptr_eq(&active, &reader.snapshot::<Owner<1>>().unwrap()));
    let dependencies = reader.sources_for(&owner).unwrap();
    assert!(dependencies.contains(&fixture.0.join("second.yaml")));
    assert!(!dependencies.contains(&fixture.0.join("first.yaml")));
    fixture.replace("corrected.yaml");
    let notification =
        std::fs::canonicalize(&fixture.0).unwrap().join("second.yaml");
    let changed = reader
        .prepare_changed_sources(&[&owner], &[notification])
        .await
        .unwrap();
    assert_eq!(changed, [std::any::TypeId::of::<Owner<1>>()]);
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, 2);
}

#[tokio::test]
async fn unreadable_syntax_keeps_known_dependencies_until_a_later_valid_read() {
    let fixture = Fixture::new();
    let owner = fixture.owner::<1>();
    let reader = reader();
    reader.initialize_owners(&[&owner]).await.unwrap();
    fixture.replace("invalid_syntax.yaml");
    assert!(reader.prepare_owners(&[&owner]).await.is_err());
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        [fixture.0.join("first.yaml")]
    );
    fixture.replace("corrected.yaml");
    let notification = std::fs::canonicalize(fixture.path()).unwrap();
    reader.prepare_changed_sources(&[&owner], &[notification]).await.unwrap();
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, 2);
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        [fixture.0.join("second.yaml")]
    );
}

#[tokio::test]
async fn missing_include_directory_does_not_block_correction_of_the_root_source()
 {
    let fixture = Fixture::new();
    let owner = fixture.owner::<1>();
    let reader = reader();
    reader.initialize_owners(&[&owner]).await.unwrap();
    fixture.replace("missing_parent.yaml");
    assert!(reader.prepare_owners(&[&owner]).await.is_err());
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        [fixture.0.join("missing/child.yaml")]
    );
    fixture.replace("corrected.yaml");
    reader
        .prepare_changed_sources(
            &[&owner],
            &[std::fs::canonicalize(fixture.path()).unwrap()],
        )
        .await
        .unwrap();
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, 2);
}

#[tokio::test]
async fn include_notification_prepares_shared_source_once() {
    let fixture = Fixture::new();
    let first = fixture.owner::<1>();
    let second = fixture.owner::<2>();
    let parses = Arc::new(AtomicU64::new(0));
    let mut reader = ConfigReader::new();
    reader.register(CountingYaml(parses.clone())).unwrap();
    reader.initialize_owners(&[&first, &second]).await.unwrap();
    let changed = std::fs::canonicalize(&fixture.0).unwrap().join("first.yaml");
    reader
        .prepare_changed_sources(&[&first, &second], &[changed])
        .await
        .unwrap();
    assert_eq!(
        parses.load(Ordering::Relaxed),
        2,
        "one parse in each batch, not one per owner"
    );
}

#[tokio::test]
async fn glob_dependencies_are_existing_files_and_are_rebuilt_on_each_source_read()
 {
    let fixture = Fixture::new();
    fixture.replace("glob.yaml");
    let parts = fixture.0.join("parts");
    std::fs::create_dir(&parts).unwrap();
    std::fs::copy(source("without.yaml"), parts.join("first.yaml")).unwrap();
    let reader = reader();
    let _: Settings = reader.read(fixture.path()).await.unwrap();
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        vec![parts.join("first.yaml")]
    );
    std::fs::copy(source("without.yaml"), parts.join("second.yaml")).unwrap();
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        [parts.join("first.yaml")]
    );
    let _: Settings = reader.read(fixture.path()).await.unwrap();
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        [parts.join("first.yaml"), parts.join("second.yaml")]
    );
    std::fs::remove_file(parts.join("first.yaml")).unwrap();
    let _: Settings = reader.read(fixture.path()).await.unwrap();
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        [parts.join("second.yaml")]
    );
    std::fs::create_dir(parts.join("directory.yaml")).unwrap();
    let _: Settings = reader.read(fixture.path()).await.unwrap();
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        [parts.join("second.yaml")]
    );
}

#[tokio::test]
async fn submitted_source_does_not_replace_live_dependency_index() {
    let fixture = Fixture::new();
    let reader = reader();
    let _: Settings = reader.read(fixture.path()).await.unwrap();
    let bytes = std::fs::read(source("replacement.yaml")).unwrap();
    let _: Settings = reader
        .parse(congif::ConfigSource { path: &fixture.path(), bytes: &bytes })
        .await
        .unwrap();
    assert_eq!(
        reader.includes_for(fixture.path()).unwrap(),
        vec![fixture.0.join("first.yaml")]
    );
}

#[tokio::test]
async fn an_owner_merging_its_includes_sees_a_changed_part_as_a_changed_config()
{
    let fixture = Fixture::new();
    fixture.replace("glob.yaml");
    let parts = fixture.0.join("parts");
    std::fs::create_dir(&parts).unwrap();
    std::fs::write(parts.join("first.yaml"), "value: 2\n").unwrap();
    std::fs::write(parts.join("second.yaml"), "value: 3\n").unwrap();
    let owner = Merging(fixture.path());
    let reader = reader();
    reader.initialize_owners(&[&owner]).await.unwrap();
    assert_eq!(reader.snapshot::<Merging>().unwrap().value, 6);
    assert!(
        reader
            .sources_for(&owner)
            .unwrap()
            .contains(&parts.join("second.yaml"))
    );
    std::fs::write(parts.join("second.yaml"), "value: 10\n").unwrap();
    let notification =
        std::fs::canonicalize(&parts).unwrap().join("second.yaml");
    let changed = reader
        .prepare_changed_sources(&[&owner], &[notification])
        .await
        .unwrap();
    assert_eq!(changed, [std::any::TypeId::of::<Merging>()]);
    assert_eq!(reader.snapshot::<Merging>().unwrap().value, 13);
}
