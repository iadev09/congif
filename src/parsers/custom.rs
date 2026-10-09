use std::path::{Path, PathBuf};

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

    /// File dependencies declared by this document. Paths are relative to the
    /// source file unless absolute. The reader tracks them without merging or
    /// executing their contents. Custom formats opt in explicitly.
    /// Built-in YAML/TOML adapters consume the reserved keys before schema
    /// decoding, so provider structs do not need metadata fields.
    fn includes(
        &self,
        _value: &mut ConfigValue,
    ) -> Result<Vec<PathBuf>, BoxError> {
        Ok(Vec::new())
    }

    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError>;

    /// Evaluate against the same caller-owned variables used for source
    /// preparation. Executable adapters may leave source text unchanged and
    /// expose these values through their language's environment API instead.
    /// Never read process environment as a substitute for this pinned input.
    async fn parse_with_variables(
        &self,
        source: ConfigSource<'_>,
        _variables: Option<&crate::Variables>,
    ) -> Result<ConfigValue, BoxError> {
        self.parse(source).await
    }
}
