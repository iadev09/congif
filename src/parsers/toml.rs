use crate::{BoxError, ConfigParser, ConfigSource, ConfigValue, async_trait};

pub struct TomlParser;

#[async_trait]
impl ConfigParser for TomlParser {
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
