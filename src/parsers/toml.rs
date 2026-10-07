use crate::{BoxError, ConfigParser, ConfigSource, ConfigValue, async_trait};

pub struct TomlParser;

#[async_trait]
impl ConfigParser for TomlParser {
    fn extensions(&self) -> &[&str] {
        &["toml"]
    }

    fn includes(
        &self,
        value: &mut ConfigValue,
    ) -> Result<Vec<std::path::PathBuf>, BoxError> {
        super::includes::paths(value)
    }

    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        Ok(::toml::from_str(std::str::from_utf8(source.bytes)?)?)
    }
}
