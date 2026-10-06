use crate::{BoxError, ConfigParser, ConfigSource, ConfigValue, async_trait};

pub struct JsonParser;

#[async_trait]
impl ConfigParser for JsonParser {
    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        Ok(serde_json::from_slice(source.bytes)?)
    }
}
