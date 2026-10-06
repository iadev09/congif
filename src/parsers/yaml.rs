use crate::{BoxError, ConfigParser, ConfigSource, ConfigValue, async_trait};

pub struct YamlParser;

#[async_trait]
impl ConfigParser for YamlParser {
    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        Ok(serde_yaml::from_slice(source.bytes)?)
    }
}
