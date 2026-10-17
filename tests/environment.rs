#![cfg(feature = "json")]

use std::any::TypeId;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use congif::{
    ConfigReader, ConfigRegistration, Configurable, Result, async_trait,
};
use dotenw::{EnvStore, Environment, MergeMode};
use serde::Deserialize;

#[derive(Deserialize, PartialEq, Eq)]
struct Settings {
    value: String,
}

struct Owner<const N: u8> {
    preparations: Arc<AtomicU64>,
}

#[async_trait]
impl<const N: u8> Configurable for Owner<N> {
    type Config = Settings;

    fn source(&self) -> congif::ConfigFile {
        congif::ConfigFile {
            path: fixture("settings.json"),
            format: congif::Format::Json,
        }
    }

    async fn configure(
        &self,
        _config: &Settings,
    ) -> Result<()> {
        Ok(())
    }

    fn validate_config(
        &self,
        _: &Settings,
    ) -> Result<()> {
        self.preparations.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/environment")
        .join(name)
}

struct Temporary(PathBuf);

struct CountLoads(Arc<AtomicU64>);

impl dotenw::EnvSource for CountLoads {
    fn files(&self) -> Vec<PathBuf> {
        vec![fixture("first.env")]
    }

    fn load(
        &self,
        _: &dotenw::SourceContext<'_>,
    ) -> dotenw::Result<dotenw::Values> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(dotenw::Values::from([("VALUE".into(), "first".into())]))
    }
}

#[tokio::test]
async fn startup_reuses_registered_environment_and_reload_refreshes_it() {
    let reads = Arc::new(AtomicU64::new(0));
    let store = EnvStore::new();
    let handle = store
        .register(
            Environment::new()
                .with_source(CountLoads(reads.clone()), MergeMode::Override),
        )
        .unwrap();
    let preparations = Arc::new(AtomicU64::new(0));
    let owner = Owner::<3> { preparations: preparations.clone() };
    let reader = ConfigReader::new()
        .with_environment_store(store)
        .with_builtin_parsers()
        .unwrap();
    reader.bind_environment::<Owner<3>>(handle).unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(reader.prepare(&owner).await.unwrap().value, "first");
    let path = fixture("settings.json");
    let bytes = std::fs::read(&path).unwrap();
    let parsed = reader
        .parse_for::<Owner<3>>(congif::ConfigSource {
            path: &path,
            bytes: &bytes,
        })
        .await
        .unwrap();
    assert_eq!(parsed.value, "first");
    reader.initialize_owners(&[&owner]).await.unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    for _ in 0..2 {
        reader
            .prepare_changed_sources(&[&owner], &[fixture("settings.json")])
            .await
            .unwrap();
    }
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    let prepared = preparations.load(Ordering::Relaxed);
    reader
        .prepare_changed_sources(&[&owner], &[fixture("first.env")])
        .await
        .unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 2);
    assert_eq!(preparations.load(Ordering::Relaxed), prepared);
    reader.prepare_changed_owners(&[&owner]).await.unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn foreign_environment_binding_does_not_replace_the_valid_binding() {
    let store = EnvStore::new();
    let values = dotenw::Values::from([("VALUE".into(), "first".into())]);
    let handle = store
        .register(
            Environment::new().with_values(values.clone(), MergeMode::Override),
        )
        .unwrap();
    let foreign = EnvStore::new()
        .register(Environment::new().with_values(values, MergeMode::Override))
        .unwrap();
    let reader = ConfigReader::new()
        .with_environment_store(store)
        .with_builtin_parsers()
        .unwrap();
    reader.bind_environment::<Owner<4>>(handle).unwrap();
    assert!(reader.bind_environment::<Owner<4>>(foreign).is_err());
    let owner = Owner::<4> { preparations: Arc::new(AtomicU64::new(0)) };
    assert_eq!(reader.prepare(&owner).await.unwrap().value, "first");
}

impl Drop for Temporary {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[tokio::test]
async fn shared_environment_prepares_owners_without_reloading_unchanged_configs()
 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = Temporary(std::env::temp_dir().join(format!(
        "congif-environment-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("application.env");
    std::fs::copy(fixture("first.env"), &path).unwrap();
    let store = EnvStore::new();
    let handle = store
        .register(
            Environment::new().with_file(path.clone(), MergeMode::Override),
        )
        .unwrap();
    let first = Owner::<1> { preparations: Arc::new(AtomicU64::new(0)) };
    let second = Owner::<2> { preparations: Arc::new(AtomicU64::new(0)) };
    let owners: [&dyn ConfigRegistration; 2] = [&first, &second];
    let reader = ConfigReader::new()
        .with_environment_store(store)
        .with_builtin_parsers()
        .unwrap();
    reader.bind_environment::<Owner<1>>(handle.clone()).unwrap();
    reader.bind_environment::<Owner<2>>(handle).unwrap();
    reader.prepare_owners(&owners).await.unwrap();
    reader.configure_registered(&first).await.unwrap();
    reader.configure_registered(&second).await.unwrap();
    let previous = reader.snapshot::<Owner<1>>().unwrap();
    let mut expected_sources =
        vec![fixture("settings.json"), std::fs::canonicalize(&path).unwrap()];
    expected_sources.sort();
    assert_eq!(reader.sources_for(&first).unwrap(), expected_sources);
    std::fs::copy(fixture("unused.env"), &path).unwrap();
    assert!(reader.prepare_changed_owners(&owners).await.unwrap().is_empty());
    assert!(Arc::ptr_eq(&previous, &reader.snapshot::<Owner<1>>().unwrap()));
    std::fs::copy(fixture("second.env"), &path).unwrap();
    assert_eq!(
        reader.prepare_changed_owners(&owners).await.unwrap(),
        vec![TypeId::of::<Owner<1>>(), TypeId::of::<Owner<2>>()]
    );
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, "second");
    assert_eq!(reader.snapshot::<Owner<2>>().unwrap().value, "second");
    assert_eq!(previous.value, "first");
}
