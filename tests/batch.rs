#![cfg(feature = "json")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use congif::{
    ConfigOwner, ConfigReader, Configurable, Error, Result, async_trait,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Settings {
    value: u64,
}

struct Owner<const N: u8> {
    path: PathBuf,
    replacement: Option<PathBuf>,
    reject: AtomicBool,
}

#[async_trait]
impl<const N: u8> Configurable for Owner<N> {
    type Config = Settings;

    async fn configure(
        &self,
        reader: &ConfigReader,
    ) -> Result<Settings> {
        let candidate = reader.read(&self.path).await?;
        // Simulate a second edit between two owners preparing the same file.
        if let Some(replacement) = &self.replacement {
            std::fs::copy(replacement, &self.path).unwrap();
        }
        Ok(candidate)
    }

    fn validate_config(
        &self,
        _: &Settings,
    ) -> Result<()> {
        if self.reject.load(Ordering::Relaxed) {
            Err(Error::validation(std::io::Error::other("rejected candidate")))
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
        Owner {
            path: self.path(),
            replacement: None,
            reject: AtomicBool::new(false),
        }
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
    let mut reader = ConfigReader::new();
    reader.register("json", congif::parsers::JsonParser).unwrap();
    reader
}
fn owners<'a>(
    first: &'a Owner<1>,
    second: &'a Owner<2>,
) -> [&'a dyn ConfigOwner; 2] {
    [first, second]
}

#[tokio::test]
async fn shared_file_is_pinned_for_all_owners_and_refreshes_next_batch() {
    let fixture = Fixture::new();
    let mut first = fixture.owner::<1>();
    first.replacement = Some(source("second.json"));
    let second = fixture.owner::<2>();
    let reader = reader();
    reader.prepare_owners(&owners(&first, &second)).await.unwrap();
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, 1);
    assert_eq!(reader.snapshot::<Owner<2>>().unwrap().value, 1);
    reader.prepare_owners(&owners(&first, &second)).await.unwrap();
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, 2);
    assert_eq!(reader.snapshot::<Owner<2>>().unwrap().value, 2);
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
