#![cfg(feature = "json")]

use std::path::PathBuf;

use congif::{ConfigFile, ConfigReader, Configurable, Result, async_trait};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Settings {
    value: u64,
}

/// Every field is for hosted consumers, so the export is the config itself.
struct Whole(PathBuf);

#[async_trait]
impl Configurable for Whole {
    type Config = Settings;

    fn source(&self) -> ConfigFile {
        ConfigFile::json(&self.0)
    }

    fn namespace(&self) -> String {
        "whole".into()
    }

    async fn configure(
        &self,
        _: &Settings,
    ) -> Result<()> {
        Ok(())
    }

    fn export_config(
        &self,
        config: &Settings,
    ) -> Result<Option<congif::ConfigValue>> {
        congif::export(config)
    }
}

#[tokio::test]
async fn a_serializable_config_exports_as_the_tree_lookup_reads() {
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    let owner = Whole(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/batch/first.json"),
    );

    reader.prepare_owners(&[&owner]).await.unwrap();

    assert_eq!(reader.config::<u64>("whole.value").unwrap(), Some(1));
    assert_eq!(reader.config::<u64>("whole.missing").unwrap(), None);
}
