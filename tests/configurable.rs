#![cfg(all(feature = "yaml", feature = "json"))]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use congif::{
    ConfigReader, ConfigStore, Configurable, Error, Result, Variables,
    async_trait,
};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct CacheSettings {
    default: String,
    stores: HashMap<String, StoreSettings>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct StoreSettings {
    driver: String,
    path: String,
}

// Configuration-only owner: no registry, cache driver or running service.
struct CacheConfiguration {
    path: PathBuf,
    variables: Variables,
}

#[async_trait]
impl Configurable for CacheConfiguration {
    type Config = CacheSettings;

    async fn configure(
        &self,
        reader: &ConfigReader,
    ) -> Result<Self::Config> {
        reader.read_with_variables(&self.path, &self.variables).await
    }

    fn validate_config(
        &self,
        config: &Self::Config,
    ) -> Result<()> {
        if !config.stores.contains_key(&config.default) {
            return Err(Error::validation(std::io::Error::other(
                "default cache store is not configured",
            )));
        }
        Ok(())
    }
}

fn owner(file: &str) -> CacheConfiguration {
    CacheConfiguration {
        path: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/configurable")
            .join(file),
        variables: Variables::from([(
            "CACHE_ROOT".into(),
            "/srv/example/cache".into(),
        )]),
    }
}

fn reader() -> ConfigReader {
    let mut reader = ConfigReader::new();
    reader.register("yaml", congif::parsers::YamlParser).unwrap();
    reader.register("json", congif::parsers::JsonParser).unwrap();
    reader
}

#[tokio::test]
async fn one_typed_owner_can_select_different_formats() {
    let reader = reader();
    let yaml: CacheSettings =
        reader.prepare(&owner("cache.yaml")).await.unwrap();
    let json: CacheSettings =
        reader.prepare(&owner("cache.json")).await.unwrap();
    assert_eq!(yaml, json);
    assert_eq!(yaml.stores["disk"].path, "/srv/example/cache/disk");
}

#[tokio::test]
async fn rejected_candidate_does_not_replace_the_active_config() {
    let reader = reader();
    let initial = reader.prepare(&owner("cache.yaml")).await.unwrap();
    let store = ConfigStore::new(initial);
    let pinned = store.snapshot();
    let rejected = reader.prepare(&owner("invalid.yaml")).await;
    assert!(matches!(rejected, Err(Error::Validation(_))));
    assert!(std::ptr::eq(pinned.as_ref(), store.snapshot().as_ref()));
}
