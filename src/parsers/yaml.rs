use crate::{BoxError, ConfigParser, ConfigSource, ConfigValue, async_trait};

pub struct YamlParser;

#[async_trait]
impl ConfigParser for YamlParser {
    /// A `${NAME}` inside a comment is documentation, not a variable.
    fn expand_variables(
        &self,
        text: &str,
        variables: &crate::Variables,
    ) -> crate::Result<String> {
        crate::variables::expand_outside_comments(
            text,
            variables,
            crate::variables::Comments::Hash,
        )
    }

    fn extensions(&self) -> &[&str] {
        &["yaml", "yml"]
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
        Ok(serde_yaml::from_slice(source.bytes)?)
    }
}
