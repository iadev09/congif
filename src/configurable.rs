use crate::{ConfigReader, Result, async_trait};
use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;

/// Provider-owned configuration preparation, independent of service registry
/// and runtime lifecycle. Every implementation declares its concrete schema.
///
/// Implementations may choose sources, formats and variables dynamically, but
/// must not publish snapshots or change live resources while preparing a
/// candidate. Domain rules and custom Serde deserializers belong to the owner.
#[async_trait]
pub trait Configurable: Send + Sync {
    type Config: Send + Sync + 'static;

    /// Files whose changes invalidate this owner's configuration.
    /// Empty means no automatic file notifications are requested.
    fn sources(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// Configuration namespace, independent of the owner's runtime identity.
    /// Defaults to the unqualified type name in snake_case; override this to
    /// expose a stable public name such as `http` instead of `http_provider`.
    /// Generic arguments do not participate in the default namespace.
    fn namespace(&self) -> String {
        let name = std::any::type_name::<Self>();
        let name = name.split('<').next().unwrap_or(name);
        let name = name.rsplit("::").next().unwrap_or(name);
        snake_case(name)
    }

    async fn configure(
        &self,
        reader: &ConfigReader,
    ) -> Result<Self::Config>;

    /// Domain validation of a prepared candidate, before publication.
    fn validate_config(
        &self,
        _config: &Self::Config,
    ) -> Result<()> {
        Ok(())
    }
}

/// Object-safe config preparation. The blanket implementation still performs
/// the owner's typed configure/validation; only storage erases the schema.
#[async_trait]
pub trait ConfigOwner: Any + Send + Sync {
    fn sources(&self) -> Vec<PathBuf>;
    async fn prepare_config(
        &self,
        reader: &ConfigReader,
    ) -> Result<Arc<dyn Any + Send + Sync>>;
}

#[async_trait]
impl<T: Configurable + 'static> ConfigOwner for T {
    fn sources(&self) -> Vec<PathBuf> {
        Configurable::sources(self)
    }
    async fn prepare_config(
        &self,
        reader: &ConfigReader,
    ) -> Result<Arc<dyn Any + Send + Sync>> {
        Ok(Arc::new(reader.prepare(self).await?))
    }
}

fn snake_case(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut chars = name.chars().peekable();
    let mut previous = None;
    while let Some(current) = chars.next() {
        if current.is_uppercase()
            && previous.is_some_and(|ch: char| {
                ch.is_lowercase()
                    || ch.is_numeric()
                    || (ch.is_uppercase()
                        && chars.peek().is_some_and(|next| next.is_lowercase()))
            })
        {
            result.push('_');
        }
        result.extend(current.to_lowercase());
        previous = Some(current);
    }
    result
}
