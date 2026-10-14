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

    /// The configuration that stands when the declared source file does not
    /// exist. `None`, the default, keeps a missing file a read error. Return a
    /// value only where absence is itself a complete declaration — no
    /// listeners, no caches — so the not-found branch is written once, here,
    /// and not in every caller. The value is validated like a read one.
    fn absent_source(&self) -> Option<Self::Config> {
        None
    }

    /// Read the candidate the reader then validates and compares. The default
    /// reads the declared source, answering an absent file with
    /// [`absent_source`](Self::absent_source). Congif records the `includes` a
    /// document names and watches them, but opens none of them: an owner whose
    /// document is completed by its includes merges them here, reading each
    /// through [`ConfigReader::read_include`] so it is parsed with the owner's
    /// environment and its own includes are recorded. The merged value is what
    /// equality compares, so a change inside an include is a changed config.
    async fn read_config(
        &self,
        reader: &ConfigReader,
    ) -> Result<Self::Config>
    where
        Self: Sized + 'static,
    {
        reader.read_source(self).await
    }

    /// Opt in to source notifications after a validated candidate is prepared.
    /// Reading a source does not, by itself, request automatic watching.
    fn as_config_observer(
        &self
    ) -> Option<&dyn ConfigObserver<Config = Self::Config>> {
        None
    }

    /// Opt in to independently watched partial sources. Ordinary owners keep
    /// their existing full-document preparation and observation path.
    fn as_partial_config_observer(
        &self
    ) -> Option<&dyn PartialConfigObserver<Config = Self::Config>> {
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
    /// A declaration consumed by a later boot constructor needs no application
    /// hook: its validated snapshot remains available in the reader.
    async fn configure(
        &self,
        _config: &Self::Config,
    ) -> Result<()> {
        Ok(())
    }

    /// Opt in to dot-path access for hosted applications and language bridges.
    /// Export only fields intended for those consumers. Called during
    /// preparation, never during lookup; typed Rust access needs no export.
    /// When the whole configuration is meant for them and it serializes,
    /// `congif::export(config)` is the one-line body.
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

/// Optional partial-source preparation and application, separate from reload.
///
/// Only these owners register partial sources. A root source, environment
/// refresh or explicit reload still prepares the complete configuration.
/// Partial reading must return a complete candidate retaining unchanged parts;
/// validation and catalog publication precede the application hook. No source
/// IO or parsing belongs in `partial_config_changed`.
/// Changed paths retain their spelling from `partial_sources`; watcher path
/// normalization is used only to match notifications to these declarations.
#[async_trait]
pub trait PartialConfigObserver: ConfigObserver {
    fn partial_sources(
        &self,
        reader: &ConfigReader,
    ) -> Result<Vec<std::path::PathBuf>>;
    async fn read_partial_config(
        &self,
        reader: &ConfigReader,
        sources: &[std::path::PathBuf],
    ) -> Result<Self::Config>;
    async fn partial_config_changed(
        &self,
        config: &Self::Config,
        sources: &[std::path::PathBuf],
    ) -> Result<()>;
}

/// The erased form of a [`Configurable`], implemented for every one of them.
///
/// Runtimes hand configurable components to the reader through this trait —
/// a provider hook returns `Option<&dyn ConfigRegistration>`, a batch takes a
/// slice of them — so heterogeneous components can be prepared together. It
/// owns no configuration; the reader does. Applications implement
/// `Configurable` and return `self` where a registration is asked for.
#[async_trait]
pub trait ConfigRegistration: Any + Send + Sync {
    fn source(&self) -> ConfigFile;
    async fn configure(
        &self,
        config: &(dyn Any + Send + Sync),
    ) -> Result<()>;
    fn observes_config(&self) -> bool;
    fn partial_sources(
        &self,
        _reader: &ConfigReader,
    ) -> Result<Option<Vec<std::path::PathBuf>>> {
        Ok(None)
    }
    async fn prepare_partial_config(
        &self,
        reader: &ConfigReader,
        _sources: &[std::path::PathBuf],
    ) -> Result<Arc<dyn Any + Send + Sync>> {
        self.prepare_config(reader).await
    }
    async fn partial_config_changed(
        &self,
        config: &(dyn Any + Send + Sync),
        _sources: &[std::path::PathBuf],
    ) -> Result<()> {
        self.config_changed(config).await
    }
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
            || self.as_partial_config_observer().is_some()
    }
    fn partial_sources(
        &self,
        reader: &ConfigReader,
    ) -> Result<Option<Vec<std::path::PathBuf>>> {
        self.as_partial_config_observer()
            .map(|observer| observer.partial_sources(reader))
            .transpose()
    }
    async fn prepare_partial_config(
        &self,
        reader: &ConfigReader,
        sources: &[std::path::PathBuf],
    ) -> Result<Arc<dyn Any + Send + Sync>> {
        let Some(observer) = self.as_partial_config_observer() else {
            return self.prepare_config(reader).await;
        };
        let config = observer.read_partial_config(reader, sources).await?;
        self.validate_config(&config)?;
        Ok(Arc::new(config))
    }
    async fn partial_config_changed(
        &self,
        config: &(dyn Any + Send + Sync),
        sources: &[std::path::PathBuf],
    ) -> Result<()> {
        let Some(observer) = self.as_partial_config_observer() else {
            return ConfigRegistration::config_changed(self, config).await;
        };
        let config = config.downcast_ref::<T::Config>().ok_or_else(|| {
            crate::Error::MissingConfig(std::any::type_name::<T>())
        })?;
        observer.partial_config_changed(config, sources).await
    }
    fn environment(&self) -> Option<&dotenw::EnvHandle> {
        Configurable::environment(self)
    }
    async fn config_changed(
        &self,
        config: &(dyn Any + Send + Sync),
    ) -> Result<()> {
        let config = config.downcast_ref::<T::Config>().ok_or_else(|| {
            crate::Error::MissingConfig(std::any::type_name::<T>())
        })?;
        if let Some(observer) = self.as_config_observer() {
            observer.config_changed(config).await
        } else if let Some(observer) = self.as_partial_config_observer() {
            observer.config_changed(config).await
        } else {
            Ok(())
        }
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

/// The value tree dot-path lookup reads, from any serializable configuration.
///
/// The body of `Configurable::export_config` for an owner whose whole config
/// is meant for hosted consumers; owners exposing a subset still build their
/// own tree.
pub fn export<T: serde::Serialize>(
    config: &T
) -> Result<Option<crate::ConfigValue>> {
    serde_value::to_value(config)
        .map(Some)
        .map_err(|error| crate::Error::Export(Box::new(error)))
}
