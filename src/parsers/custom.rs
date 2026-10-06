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
    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> std::result::Result<ConfigValue, BoxError>;
}
