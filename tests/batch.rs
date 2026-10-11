#![cfg(feature = "json")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use congif::{
    ConfigReader, ConfigRegistration, Configurable, Error, Result, async_trait,
};
use serde::Deserialize;

struct CountingJson(std::sync::Arc<AtomicU64>);

#[async_trait]
impl congif::ConfigParser for CountingJson {
    fn extensions(&self) -> &[&str] {
        &["json"]
    }

    async fn parse(
        &self,
        source: congif::ConfigSource<'_>,
    ) -> std::result::Result<congif::ConfigValue, congif::BoxError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        congif::ConfigParser::parse(&congif::parsers::JsonParser, source).await
    }
}

#[derive(Deserialize, PartialEq)]
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

    fn source(&self) -> congif::ConfigFile {
        congif::ConfigFile {
            path: self.path.clone(),
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
        if self.reject.load(Ordering::Relaxed) {
            Err(Error::invalid("rejected candidate"))
        } else {
            Ok(())
        }
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "congif-batch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let fixture = Self(dir);
        fixture.reset("first.json");
        fixture
    }
    fn path(&self) -> PathBuf {
        self.0.join("settings.json")
    }
    fn reset(
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
        .join("tests/fixtures/batch")
        .join(name)
}
fn reader() -> ConfigReader {
    ConfigReader::new().with_builtin_parsers().unwrap()
}
fn owners<'a>(
    first: &'a Owner<1>,
    second: &'a Owner<2>,
) -> [&'a dyn ConfigRegistration; 2] {
    [first, second]
}

#[tokio::test]
async fn shared_file_is_pinned_for_all_owners_and_refreshes_next_batch() {
    let fixture = Fixture::new();
    let first = fixture.owner::<1>();
    let second = fixture.owner::<2>();
    let parses = std::sync::Arc::new(AtomicU64::new(0));
    let reader = ConfigReader::new();
    reader.register(CountingJson(parses.clone())).unwrap();
    reader.prepare_owners(&owners(&first, &second)).await.unwrap();
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, 1);
    assert_eq!(reader.snapshot::<Owner<2>>().unwrap().value, 1);
    assert_eq!(
        parses.load(Ordering::Relaxed),
        1,
        "one syntax parse for both owners"
    );
    fixture.reset("second.json");
    reader.prepare_owners(&owners(&first, &second)).await.unwrap();
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, 2);
    assert_eq!(reader.snapshot::<Owner<2>>().unwrap().value, 2);
    assert_eq!(
        parses.load(Ordering::Relaxed),
        2,
        "the next batch parses the new source once"
    );
}

#[tokio::test]
async fn failed_validation_or_read_keeps_every_previous_snapshot() {
    let fixture = Fixture::new();
    let first = fixture.owner::<1>();
    let second = fixture.owner::<2>();
    let reader = reader();
    reader.prepare_owners(&owners(&first, &second)).await.unwrap();
    let original = reader.snapshot::<Owner<1>>().unwrap();
    fixture.reset("second.json");
    second.reject.store(true, Ordering::Relaxed);
    assert!(reader.prepare_owners(&owners(&first, &second)).await.is_err());
    assert!(std::sync::Arc::ptr_eq(
        &original,
        &reader.snapshot::<Owner<1>>().unwrap()
    ));
    assert_eq!(reader.snapshot::<Owner<2>>().unwrap().value, 1);
    std::fs::remove_file(fixture.path()).unwrap();
    assert!(reader.prepare_owners(&owners(&first, &second)).await.is_err());
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, 1);
    assert_eq!(reader.snapshot::<Owner<2>>().unwrap().value, 1);
}
