use crate::{BoxError, ConfigParser, ConfigSource, ConfigValue, async_trait};

pub struct JsonParser;

#[async_trait]
impl ConfigParser for JsonParser {
    fn expand_variables(
        &self,
        text: &str,
        variables: &crate::Variables,
    ) -> crate::Result<String> {
        crate::variables::expand_json(text, variables)
    }

    fn extensions(&self) -> &[&str] {
        &["json"]
    }

    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        Ok(serde_json::from_slice(source.bytes)?)
    }
}
