use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::de::DeserializeOwned;

use crate::{
    ConfigParser, ConfigRegistration, ConfigSource, ConfigStore, Configurable,
    Error, Result, Variables,
};

type SourceBytes = Arc<tokio::sync::Mutex<HashMap<PathBuf, Arc<[u8]>>>>;
type ParsedSources = Arc<
    tokio::sync::Mutex<
        HashMap<(PathBuf, usize, Arc<[u8]>), crate::ConfigValue>,
    >,
>;

#[derive(Clone)]
struct ConfigEntry {
    typed: Arc<dyn Any + Send + Sync>,
    exported: Option<Arc<(String, crate::ConfigValue)>>,
    applied: Option<Arc<dyn Any + Send + Sync>>,
}

/// An exact prepared generation, pinned before applying runtime resources.
/// Confirm this token only after that generation was successfully applied.
#[derive(Clone)]
pub struct PreparedConfig {
    owner: TypeId,
    value: Arc<dyn Any + Send + Sync>,
    reader: Arc<()>,
}

impl ConfigEntry {
    fn needs_application(
        &self,
        owner: &dyn ConfigRegistration,
    ) -> bool {
        self.applied.as_ref().is_none_or(|applied| {
            !Arc::ptr_eq(applied, &self.typed)
                && !owner.same_config(applied.as_ref(), self.typed.as_ref())
        })
    }
}

enum EnvironmentRefresh<'a> {
    None,
    All,
    Files(&'a [PathBuf]),
}

/// Parsers are selected explicitly, individually or with `with_builtin_parsers`.
/// Paths are used as supplied: no implicit
/// root, environment substitution, merging, watcher or reload timer. Batch
/// preparation pins file bytes and publishes one atomic owner-keyed catalog.
pub struct ConfigReader {
    identity: Arc<()>,
    parsers: HashMap<String, Arc<dyn ConfigParser>>,
    configs: ConfigStore<HashMap<TypeId, ConfigEntry>>,
    preparation: tokio::sync::Mutex<()>,
    sources: Option<SourceBytes>,
    parsed_sources: Option<ParsedSources>,
    environments: dotenw::EnvStore,
    environment_bindings: Arc<RwLock<HashMap<TypeId, dotenw::EnvHandle>>>,
    environment_catalog: Option<Arc<dotenw::CatalogSnapshot>>,
    variables: Option<dotenw::Snapshot>,
}

impl Default for ConfigReader {
    fn default() -> Self {
        Self {
            identity: Arc::new(()),
            parsers: HashMap::new(),
            configs: ConfigStore::new(HashMap::new()),
            preparation: tokio::sync::Mutex::new(()),
            sources: None,
            parsed_sources: None,
            environments: dotenw::EnvStore::new(),
            environment_bindings: Arc::new(RwLock::new(HashMap::new())),
            environment_catalog: None,
            variables: None,
        }
    }
}

impl ConfigReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_environment_store(
        mut self,
        environments: dotenw::EnvStore,
    ) -> Self {
        self.environments = environments;
        self
    }

    pub fn environments(&self) -> &dotenw::EnvStore {
        &self.environments
    }

    /// Associate an owner's configuration with an environment held by this reader.
    /// The provider need not store the handle or implement an environment hook.
    pub fn bind_environment<C: Configurable + 'static>(
        &self,
        environment: dotenw::EnvHandle,
    ) -> Result<()> {
        self.environments.snapshot().get(&environment)?;
        self.environment_bindings
            .write()
            .map_err(|_| {
                Error::msg("configuration environment bindings poisoned")
            })?
            .insert(TypeId::of::<C>(), environment);
        Ok(())
    }

    fn environment_for(
        &self,
        owner: TypeId,
    ) -> Result<Option<dotenw::EnvHandle>> {
        Ok(self
            .environment_bindings
            .read()
            .map_err(|_| {
                Error::msg("configuration environment bindings poisoned")
            })?
            .get(&owner)
            .cloned())
    }

    /// Configuration files plus the bound environment's complete file ancestry.
    /// Watchers use this list without asking providers to own environment state.
    pub fn sources_for(
        &self,
        owner: &dyn ConfigRegistration,
    ) -> Result<Vec<PathBuf>> {
        let mut paths = vec![owner.source().path];
        if let Some(environment) = owner
            .environment()
            .cloned()
            .or(self.environment_for(owner.type_id())?)
        {
            self.environments.snapshot().get(&environment)?;
            paths.extend(environment.sources()?);
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// Add all built-in parsers enabled by Cargo features.
    /// Existing custom mappings are never silently overwritten.
    pub fn with_builtin_parsers(mut self) -> Result<Self> {
        let builtins = crate::parsers::builtin_parsers();
        for extension in builtins.keys() {
            if self.parsers.contains_key(extension) {
                return Err(Error::DuplicateParser(extension.clone()));
            }
        }
        self.parsers.extend(builtins);
        Ok(self)
    }

    /// Prepare every affected owner against one file-byte generation. A failed
    /// read, parse or validation leaves the entire published catalog unchanged.
    /// Runtime resource changes are deliberately outside this transaction.
    pub async fn prepare_owners(
        &self,
        owners: &[&dyn ConfigRegistration],
    ) -> Result<()> {
        self.prepare_changed_owners(owners).await.map(|_| ())
    }

    /// Prepare initial configs from already-registered environment snapshots.
    /// Startup does not refresh environment sources a second time.
    pub async fn initialize_owners(
        &self,
        owners: &[&dyn ConfigRegistration],
    ) -> Result<()> {
        self.prepare_owner_batch(owners, EnvironmentRefresh::None)
            .await
            .map(|_| ())
    }

    /// Env preparation precedes typed config preparation; unchanged configs
    /// retain their previous value and do not request a runtime reload.
    pub async fn prepare_changed_owners(
        &self,
        owners: &[&dyn ConfigRegistration],
    ) -> Result<Vec<TypeId>> {
        self.prepare_owner_batch(owners, EnvironmentRefresh::All).await
    }

    /// Prepare file-invalidated owners using current environment snapshots,
    /// refreshing only environment sources among the changed paths.
    pub async fn prepare_changed_sources(
        &self,
        owners: &[&dyn ConfigRegistration],
        paths: &[PathBuf],
    ) -> Result<Vec<TypeId>> {
        self.prepare_owner_batch(owners, EnvironmentRefresh::Files(paths)).await
    }

    async fn prepare_owner_batch(
        &self,
        owners: &[&dyn ConfigRegistration],
        refresh_environment: EnvironmentRefresh<'_>,
    ) -> Result<Vec<TypeId>> {
        let _guard = self.preparation.lock().await;
        let mut bindings = self
            .environment_bindings
            .read()
            .map_err(|_| {
                Error::msg("configuration environment bindings poisoned")
            })?
            .clone();
        for owner in owners {
            if let Some(environment) = owner.environment() {
                self.environments.snapshot().get(environment)?;
                bindings.insert((*owner).type_id(), environment.clone());
            }
        }
        let environments: Vec<_> = owners
            .iter()
            .filter_map(|owner| bindings.get(&(*owner).type_id()).cloned())
            .collect();
        for environment in &environments {
            self.environments.snapshot().get(environment)?;
        }
        let mut changed_environments = Vec::new();
        match refresh_environment {
            EnvironmentRefresh::All if !environments.is_empty() => {
                tokio::task::spawn_blocking(move || {
                    dotenw::EnvHandle::refresh_many(&environments)
                })
                .await
                .map_err(Error::validation)??;
            }
            EnvironmentRefresh::Files(paths) if !paths.is_empty() => {
                let paths = paths.to_vec();
                let store = self.environments.clone();
                let refreshed = tokio::task::spawn_blocking(move || {
                    store.refresh_files(&paths)
                })
                .await
                .map_err(Error::validation)??;
                if let Some((_, changed)) = refreshed {
                    changed_environments = changed;
                }
            }
            _ => {}
        }
        let reader = Self {
            identity: self.identity.clone(),
            parsers: self.parsers.clone(),
            sources: Some(Arc::new(tokio::sync::Mutex::new(HashMap::new()))),
            parsed_sources: Some(Arc::new(tokio::sync::Mutex::new(
                HashMap::new(),
            ))),
            configs: ConfigStore::new(self.configs.snapshot().as_ref().clone()),
            preparation: tokio::sync::Mutex::new(()),
            environments: self.environments.clone(),
            environment_bindings: Arc::new(RwLock::new(bindings.clone())),
            environment_catalog: Some(Arc::new(self.environments.snapshot())),
            variables: None,
        };
        let mut candidate = self.configs.snapshot().as_ref().clone();
        let mut changed = Vec::new();
        let mut updated = false;
        let mut pending = Vec::new();
        for owner in owners {
            let id = (*owner).type_id();
            if let EnvironmentRefresh::Files(paths) = &refresh_environment
                && candidate.contains_key(&id)
            {
                let config_sources = [owner.source().path];
                let mut config_changed = false;
                for source in config_sources {
                    let parent = source
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new("."));
                    let parent = tokio::fs::canonicalize(parent)
                        .await
                        .map_err(|source_error| Error::Read {
                            path: parent.to_owned(),
                            source: source_error,
                        })?;
                    let Some(name) = source.file_name() else {
                        config_changed = true;
                        break;
                    };
                    if paths.iter().any(|path| path == &parent.join(name)) {
                        config_changed = true;
                        break;
                    }
                }
                let environment_changed =
                    reader.environment_for(id)?.is_some_and(|environment| {
                        changed_environments.contains(&environment.id())
                    });
                if !config_changed && !environment_changed {
                    if candidate[&id].needs_application(*owner) {
                        changed.push(id);
                    }
                    tracing::debug!(owner = ?id, "source notification unchanged; configuration preparation skipped");
                    continue;
                }
            }
            pending.push(*owner);
        }
        // Pin every declared source before deserializing any component.
        // Each component still owns its schema and domain validation.
        for owner in &pending {
            let file = owner.source();
            let parser = reader.format_parser(&file.format, &file.path)?;
            // An absent file is the owner's to answer for in `prepare`.
            let bytes = match reader.read_bytes(&file.path).await {
                Err(Error::Read { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    continue;
                }
                other => other?,
            };
            let variables = match reader.environment_for((*owner).type_id())? {
                Some(environment) => Some(
                    reader
                        .environment_catalog
                        .as_ref()
                        .expect("batch environment catalog")
                        .get(&environment)?,
                ),
                None => None,
            };
            reader
                .syntax(
                    parser,
                    ConfigSource { path: &file.path, bytes: &bytes },
                    variables.as_deref(),
                )
                .await?;
        }
        for owner in pending {
            let id = owner.type_id();
            let prepared = owner.prepare_config(&reader).await?;
            if candidate.get(&id).is_some_and(|previous| {
                owner.same_config(previous.typed.as_ref(), prepared.as_ref())
            }) {
                if candidate[&id].needs_application(owner) {
                    changed.push(id);
                }
                tracing::debug!(owner = ?id, "prepared configuration unchanged; existing snapshot retained");
                continue;
            }
            let exported = owner.exported_config(prepared.as_ref())?;
            if let Some((namespace, _)) = &exported {
                if namespace.is_empty() || namespace.contains('.') {
                    return Err(Error::InvalidNamespace(namespace.clone()));
                }
                if candidate.iter().any(|(other, entry)| {
                    *other != id
                        && entry
                            .exported
                            .as_ref()
                            .is_some_and(|exported| &exported.0 == namespace)
                }) {
                    return Err(Error::DuplicateNamespace(namespace.clone()));
                }
            }
            let applied =
                candidate.get(&id).and_then(|entry| entry.applied.clone());
            let requires_application = applied.as_ref().is_none_or(|applied| {
                !owner.same_config(applied.as_ref(), prepared.as_ref())
            });
            candidate.insert(
                id,
                ConfigEntry {
                    typed: prepared,
                    exported: exported.map(Arc::new),
                    applied,
                },
            );
            updated = true;
            if requires_application {
                changed.push(id);
            }
            tracing::debug!(owner = ?id, "prepared configuration changed");
            // Later owners may resolve already-prepared dependency configs.
            reader.configs.replace(candidate.clone(), |_| Ok(()))?;
        }
        if updated {
            self.configs.replace(candidate, |_| Ok(()))?;
        }
        // Keep submitted-source parsing on the same environment selected by
        // the successfully prepared owner; failed batches do not rebind it.
        *self.environment_bindings.write().map_err(|_| {
            Error::msg("configuration environment bindings poisoned")
        })? = bindings;
        Ok(changed)
    }

    /// Pin the candidate that an application is about to apply. Owners without
    /// a prepared config have no token. This does not mark resources as applied.
    pub fn prepared_config(
        &self,
        owner: &dyn ConfigRegistration,
    ) -> Option<PreparedConfig> {
        let id = owner.type_id();
        self.configs.snapshot().get(&id).map(|entry| PreparedConfig {
            owner: id,
            value: entry.typed.clone(),
            reader: self.identity.clone(),
        })
    }

    /// Deliver one pinned candidate to its opt-in observer. Failed delivery
    /// leaves it unapplied so a subsequent invalidation can retry it. Preparing
    /// a batch alone never executes this hook or changes runtime resources.
    pub async fn notify_changed(
        &self,
        owner: &dyn ConfigRegistration,
    ) -> Result<()> {
        if !owner.observes_config() {
            return Err(Error::msg("configuration owner has no observer"));
        }
        let prepared = self.prepared_config(owner).ok_or_else(|| {
            Error::msg("configuration owner has not been prepared")
        })?;
        owner.config_changed(prepared.value.as_ref()).await?;
        self.confirm_applied(prepared).await
    }

    /// Apply a prepared initial configuration after all registrations have
    /// been read and validated. No source I/O occurs during this call.
    pub async fn configure_registered(
        &self,
        registration: &dyn ConfigRegistration,
    ) -> Result<()> {
        let prepared = self
            .prepared_config(registration)
            .ok_or_else(|| Error::msg("configuration has not been prepared"))?;
        registration.configure(prepared.value.as_ref()).await?;
        self.confirm_applied(prepared).await
    }

    /// Acknowledge successful application, sharing the existing Arc rather
    /// than copying configuration. A stale token never marks a newer candidate
    /// applied. Failed application must not call this method.
    pub async fn confirm_applied(
        &self,
        prepared: PreparedConfig,
    ) -> Result<()> {
        if !Arc::ptr_eq(&self.identity, &prepared.reader) {
            return Err(Error::ForeignPreparedConfig);
        }
        let _guard = self.preparation.lock().await;
        let mut catalog = self.configs.snapshot().as_ref().clone();
        if let Some(entry) = catalog.get_mut(&prepared.owner) {
            entry.applied = Some(prepared.value);
            self.configs.replace(catalog, |_| Ok(()))?;
        }
        Ok(())
    }

    /// Pin the prepared config under its owner type, not under its schema type.
    pub fn snapshot<C: Configurable + 'static>(
        &self
    ) -> Result<Arc<C::Config>> {
        self.configs
            .snapshot()
            .get(&TypeId::of::<C>())
            .map(|entry| entry.typed.clone())
            .and_then(|value| value.downcast::<C::Config>().ok())
            .ok_or_else(|| Error::MissingConfig(std::any::type_name::<C>()))
    }

    /// Typed dot-path access for Rust-hosted applications and language bridges.
    /// Prefer [`Self::snapshot`] when the owner is known in Rust code.
    /// Owners export a value tree during preparation; lookup performs no file
    /// reads or serialization. Missing keys return `None`; incompatible values
    /// return an error containing the key and requested type. No default is supplied.
    pub fn config<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>> {
        self.value(key)?
            .map(|value| {
                value.deserialize_into().map_err(|source| Error::Lookup {
                    key: key.to_owned(),
                    expected: std::any::type_name::<T>(),
                    source,
                })
            })
            .transpose()
    }

    /// Read an exported field from one pinned configuration generation.
    pub fn value(
        &self,
        key: &str,
    ) -> Result<Option<crate::ConfigValue>> {
        if key.is_empty() || key.split('.').any(str::is_empty) {
            return Err(Error::InvalidKey(key.to_owned()));
        }
        let mut parts = key.split('.');
        let namespace = parts.next().expect("nonempty key");
        let catalog = self.configs.snapshot();
        let Some(exported) = catalog
            .values()
            .filter_map(|entry| entry.exported.as_ref())
            .find(|exported| exported.0 == namespace)
        else {
            return Ok(None);
        };
        let mut value = &exported.1;
        for part in parts {
            value = match value {
                crate::ConfigValue::Map(map) => match map
                    .get(&crate::ConfigValue::String(part.to_owned()))
                {
                    Some(value) => value,
                    None => return Ok(None),
                },
                crate::ConfigValue::Seq(values) => match part
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| values.get(index))
                {
                    Some(value) => value,
                    None => return Ok(None),
                },
                _ => return Ok(None),
            };
        }
        Ok(Some(value.clone()))
    }

    async fn read_bytes(
        &self,
        path: &Path,
    ) -> Result<Arc<[u8]>> {
        let Some(sources) = &self.sources else {
            return tokio::fs::read(path).await.map(Arc::from).map_err(
                |source| Error::Read { path: path.to_owned(), source },
            );
        };
        let key = tokio::fs::canonicalize(path)
            .await
            .map_err(|source| Error::Read { path: path.to_owned(), source })?;
        let mut sources = sources.lock().await;
        if let Some(bytes) = sources.get(&key) {
            tracing::trace!(path = %key.display(), "configuration source reused within preparation batch");
            return Ok(bytes.clone());
        }
        let bytes: Arc<[u8]> = tokio::fs::read(path)
            .await
            .map(Arc::from)
            .map_err(|source| Error::Read { path: path.to_owned(), source })?;
        sources.insert(key, bytes.clone());
        Ok(bytes)
    }

    /// Prepare and validate an owner's concrete configuration. This does not
    /// replace an active snapshot, refresh environment sources, or boot/reload
    /// any runtime resources. Batch reload refreshes environments explicitly.
    pub async fn prepare<C: Configurable + ?Sized + 'static>(
        &self,
        owner: &C,
    ) -> Result<C::Config> {
        let environment = owner
            .environment()
            .cloned()
            .or(self.environment_for(TypeId::of::<C>())?);
        let candidate = if let Some(environment) = environment {
            self.environments.snapshot().get(&environment)?;
            let variables = match &self.environment_catalog {
                Some(catalog) => catalog.get(&environment)?,
                None => environment.snapshot()?,
            };
            let context = Self {
                identity: self.identity.clone(),
                parsers: self.parsers.clone(),
                sources: self.sources.clone(),
                parsed_sources: self.parsed_sources.clone(),
                configs: ConfigStore::new(
                    self.configs.snapshot().as_ref().clone(),
                ),
                preparation: tokio::sync::Mutex::new(()),
                environments: self.environments.clone(),
                environment_bindings: self.environment_bindings.clone(),
                environment_catalog: self.environment_catalog.clone(),
                variables: Some(variables),
            };
            context.read_file(&owner.source()).await
        } else {
            self.read_file(&owner.source()).await
        };
        let candidate = match candidate {
            Err(Error::Read { path, source })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                owner.absent_source().ok_or(Error::Read { path, source })?
            }
            other => other?,
        };
        owner.validate_config(&candidate)?;
        Ok(candidate)
    }

    /// Register a parser under all of its declared extensions.
    /// Failed validation or a conflict leaves the registry unchanged.
    pub fn register<P: ConfigParser + 'static>(
        &mut self,
        parser: P,
    ) -> Result<&mut Self> {
        let declared = parser.extensions();
        if declared.is_empty() {
            return Err(Error::MissingExtensions);
        }
        let mut extensions = Vec::with_capacity(declared.len());
        for extension in declared {
            if extension.is_empty()
                || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                return Err(Error::InvalidExtension((*extension).to_owned()));
            }
            let extension = extension.to_ascii_lowercase();
            if self.parsers.contains_key(&extension) {
                return Err(Error::DuplicateParser(extension));
            }
            if !extensions.contains(&extension) {
                extensions.push(extension);
            }
        }
        let parser: Arc<dyn ConfigParser> = Arc::new(parser);
        for extension in extensions {
            self.parsers.insert(extension, parser.clone());
        }
        Ok(self)
    }

    fn parser(
        &self,
        path: &Path,
    ) -> Result<&dyn ConfigParser> {
        path.extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| {
                self.parsers.get(&extension.to_ascii_lowercase())
            })
            .map(Arc::as_ref)
            .ok_or_else(|| Error::UnsupportedFormat { path: path.to_owned() })
    }

    fn format_parser(
        &self,
        format: &crate::Format,
        path: &Path,
    ) -> Result<&dyn ConfigParser> {
        self.parsers
            .get(&format.key().to_ascii_lowercase())
            .map(Arc::as_ref)
            .ok_or_else(|| Error::UnsupportedFormat { path: path.to_owned() })
    }

    pub async fn read_file<T: DeserializeOwned>(
        &self,
        file: &crate::ConfigFile,
    ) -> Result<T> {
        let parser = self.format_parser(&file.format, &file.path)?;
        let bytes = self.read_bytes(&file.path).await?;
        self.decode(
            parser,
            ConfigSource { path: &file.path, bytes: &bytes },
            self.variables.as_deref(),
        )
        .await
    }

    pub async fn read<T: DeserializeOwned>(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<T> {
        let path = path.as_ref();
        // Resolve before I/O: an unsupported format is not opened/executed.
        let parser = self.parser(path)?;
        let bytes = self.read_bytes(path).await?;
        self.decode(
            parser,
            ConfigSource { path, bytes: &bytes },
            self.variables.as_deref(),
        )
        .await
    }

    /// Read a file, expand caller-supplied variables, then parse the result.
    pub async fn read_with_variables<T: DeserializeOwned>(
        &self,
        path: impl AsRef<Path>,
        variables: &Variables,
    ) -> Result<T> {
        let path = path.as_ref();
        let parser = self.parser(path)?;
        let bytes = self.read_bytes(path).await?;
        self.decode(
            parser,
            ConfigSource { path, bytes: &bytes },
            Some(variables),
        )
        .await
    }

    /// Decode an already-read source without filesystem access.
    pub async fn parse<T: DeserializeOwned>(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<T> {
        self.decode(
            self.parser(source.path)?,
            source,
            self.variables.as_deref(),
        )
        .await
    }

    /// Expand `$NAME` and `${NAME}` in source text, then parse and apply the schema.
    /// The adapter controls escaping; no environment lookup or recursive expansion.
    pub async fn parse_with_variables<T: DeserializeOwned>(
        &self,
        source: ConfigSource<'_>,
        variables: &Variables,
    ) -> Result<T> {
        self.decode(self.parser(source.path)?, source, Some(variables)).await
    }

    /// Parse submitted source using an owner's bound environment without publishing.
    /// Domain validation remains the owner's responsibility, as with `parse`.
    pub async fn parse_for<C: Configurable + 'static>(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<C::Config>
    where
        C::Config: DeserializeOwned,
    {
        let variables = if let Some(environment) =
            self.environment_for(TypeId::of::<C>())?
        {
            self.environments.snapshot().get(&environment)?;
            Some(match &self.environment_catalog {
                Some(catalog) => catalog.get(&environment)?,
                None => environment.snapshot()?,
            })
        } else {
            None
        };
        self.decode(self.parser(source.path)?, source, variables.as_deref())
            .await
    }

    async fn decode<T: DeserializeOwned>(
        &self,
        parser: &dyn ConfigParser,
        source: ConfigSource<'_>,
        variables: Option<&Variables>,
    ) -> Result<T> {
        let value = self.syntax(parser, source, variables).await?;
        T::deserialize(value).map_err(|error| Error::Deserialize {
            path: source.path.to_owned(),
            source: error,
        })
    }

    async fn syntax(
        &self,
        parser: &dyn ConfigParser,
        source: ConfigSource<'_>,
        variables: Option<&Variables>,
    ) -> Result<crate::ConfigValue> {
        let expanded;
        let source = if let Some(variables) = variables {
            let text = std::str::from_utf8(source.bytes).map_err(|error| {
                Error::Parse {
                    path: source.path.to_owned(),
                    source: Box::new(error),
                }
            })?;
            expanded = parser.expand_variables(text, variables)?;
            ConfigSource { path: source.path, bytes: expanded.as_bytes() }
        } else {
            source
        };
        let mut parsed = match &self.parsed_sources {
            Some(cache) => Some(cache.lock().await),
            None => None,
        };
        // Aliases of the same registered adapter can share syntax; different
        // adapters for one file must never share it. Expanded bytes separate
        // different environment/explicit-variable contexts within the batch.
        let parser_id = parser as *const dyn ConfigParser as *const () as usize;
        let key = (
            source.path.to_owned(),
            parser_id,
            Arc::<[u8]>::from(source.bytes),
        );
        if let Some(value) = parsed.as_ref().and_then(|cache| cache.get(&key)) {
            tracing::trace!(path = %source.path.display(), "configuration syntax reused within preparation batch");
            return Ok(value.clone());
        }
        let value = parser.parse(source).await.map_err(|error| {
            Error::Parse { path: source.path.to_owned(), source: error }
        })?;
        tracing::trace!(path = %source.path.display(), "configuration source parsed");
        if let Some(cache) = parsed.as_mut() {
            cache.insert(key, value.clone());
        }
        Ok(value)
    }
}
