use std::path::Path;

use crate::{BoxError, ConfigValue, async_trait};

/// Original source path and bytes; adapters may carry their own execution context.
#[derive(Clone, Copy)]
pub struct ConfigSource<'a> {
    pub path: &'a Path,
    pub bytes: &'a [u8],
}

/// Implement this contract in an external crate to register a custom parser.
/// No built-in format feature is required.
#[async_trait]
pub trait ConfigParser: Send + Sync {
    /// Expand caller-owned variables before parsing. Override for format-specific escaping.
    fn expand_variables(
        &self,
        text: &str,
        variables: &crate::Variables,
    ) -> crate::Result<String> {
        crate::variables::expand(text, variables)
    }

    /// File extensions this parser handles, without a leading dot.
    /// Explicit formats select a registered key; direct path reads use its extension.
    fn extensions(&self) -> &[&str];

    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError>;
}
