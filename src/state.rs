use serde::de::DeserializeOwned;

use crate::{ConfigReader, Result};

/// Application-owned access to a common reader. No runtime state or lifecycle
/// is required; implement this on the application's state or context.
pub trait HasConfigReader {
    fn config_reader(&self) -> &ConfigReader;

    /// Resolve an exported dot path through this application's reader.
    /// Missing keys are `None`; incompatible values are errors. No default.
    fn get_config<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>>
    where
        Self: Sized,
    {
        self.config_reader().config(key)
    }
}
