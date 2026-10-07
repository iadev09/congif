use crate::{ConfigFile, ConfigReader, Result, async_trait};
use std::any::Any;
use std::sync::Arc;

/// Typed application configuration. Congif reads and validates the declared
/// source before the lifecycle invokes `configure` with the prepared value.
#[async_trait]
pub trait Configurable: Send + Sync {
    /// Congif compares prepared and applied configurations using this type's
    /// equality, without asking the provider to retain or compare snapshots.
    type Config: serde::de::DeserializeOwned + PartialEq + Send + Sync + 'static;

    /// One source with an explicit parser format. Reading is owned by the reader.
    fn source(&self) -> ConfigFile;

    /// Opt in to source notifications after a validated candidate is prepared.
    /// Reading a source does not, by itself, request automatic watching.
    fn as_config_observer(
        &self
    ) -> Option<&dyn ConfigObserver<Config = Self::Config>> {
        None
    }

    /// Select a registered environment for configuration preparation.
    /// `None` uses the reader's application-owned binding, if any. This is not
    /// the environment supplied to a child process or hosted application.
    fn environment(&self) -> Option<&dotenw::EnvHandle> {
        None
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

    /// Apply the prepared startup config. Reading and validation have finished;
    /// this hook may initialize runtime resources without parsing again.
    async fn configure(
        &self,
        config: &Self::Config,
    ) -> Result<()>;

    /// Opt in to dot-path access for hosted applications and language bridges.
    /// Export only fields intended for those consumers. Called during
    /// preparation, never during lookup; typed Rust access needs no export.
    fn export_config(
        &self,
        _config: &Self::Config,
    ) -> Result<Option<crate::ConfigValue>> {
        Ok(None)
    }

    /// Validate semantic consistency of a prepared candidate before publication.
    /// Serde checks types and declared deserialization rules; this hook checks
    /// interdependent parameters and runtime-specific constraints that those
    /// rules cannot express. Config-local checks may be delegated to the config's
    /// own validation method. Validation must not change live runtime resources.
    fn validate_config(
        &self,
        _config: &Self::Config,
    ) -> Result<()> {
        Ok(())
    }
}

/// Applies an already-prepared configuration in response to a source change.
/// The runtime calls this outside the native watcher thread. It does not read
/// files again and is independent of an explicit/manual reload request.
#[async_trait]
pub trait ConfigObserver: Configurable {
    async fn config_changed(
        &self,
        config: &Self::Config,
    ) -> Result<()>;
}

/// Erased lifecycle adapter for heterogeneous registrations. This does not own
/// config storage; ConfigReader does. Applications implement Configurable,
/// not this infrastructure contract.
#[doc(hidden)]
#[async_trait]
pub trait ConfigRegistration: Any + Send + Sync {
    fn source(&self) -> ConfigFile;
    async fn configure(
        &self,
        config: &(dyn Any + Send + Sync),
    ) -> Result<()>;
    fn observes_config(&self) -> bool;
    fn environment(&self) -> Option<&dotenw::EnvHandle>;
    async fn config_changed(
        &self,
        config: &(dyn Any + Send + Sync),
    ) -> Result<()>;
    fn exported_config(
        &self,
        _config: &(dyn Any + Send + Sync),
    ) -> Result<Option<(String, crate::ConfigValue)>> {
        Ok(None)
    }
    fn same_config(
        &self,
        previous: &(dyn Any + Send + Sync),
        candidate: &(dyn Any + Send + Sync),
    ) -> bool;
    async fn prepare_config(
        &self,
        reader: &ConfigReader,
    ) -> Result<Arc<dyn Any + Send + Sync>>;
}

#[async_trait]
impl<T: Configurable + 'static> ConfigRegistration for T {
    fn source(&self) -> ConfigFile {
        Configurable::source(self)
    }
    async fn configure(
        &self,
        config: &(dyn Any + Send + Sync),
    ) -> Result<()> {
        let config = config.downcast_ref::<T::Config>().ok_or_else(|| {
            crate::Error::MissingConfig(std::any::type_name::<T>())
        })?;
        Configurable::configure(self, config).await
    }
    fn observes_config(&self) -> bool {
        self.as_config_observer().is_some()
    }
    fn environment(&self) -> Option<&dotenw::EnvHandle> {
        Configurable::environment(self)
    }
    async fn config_changed(
        &self,
        config: &(dyn Any + Send + Sync),
    ) -> Result<()> {
        let Some(observer) = self.as_config_observer() else { return Ok(()) };
        let config = config.downcast_ref::<T::Config>().ok_or_else(|| {
            crate::Error::MissingConfig(std::any::type_name::<T>())
        })?;
        observer.config_changed(config).await
    }
    fn exported_config(
        &self,
        config: &(dyn Any + Send + Sync),
    ) -> Result<Option<(String, crate::ConfigValue)>> {
        let config = config.downcast_ref::<T::Config>().ok_or_else(|| {
            crate::Error::MissingConfig(std::any::type_name::<T>())
        })?;
        Ok(self.export_config(config)?.map(|value| (self.namespace(), value)))
    }
    fn same_config(
        &self,
        previous: &(dyn Any + Send + Sync),
        candidate: &(dyn Any + Send + Sync),
    ) -> bool {
        match (
            previous.downcast_ref::<T::Config>(),
            candidate.downcast_ref::<T::Config>(),
        ) {
            (Some(previous), Some(candidate)) => previous == candidate,
            _ => false,
        }
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
