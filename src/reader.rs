use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::{
    ConfigOwner, ConfigParser, ConfigSource, ConfigStore, Configurable, Error,
    Result, Variables,
};

/// Parser registration is explicit. Paths are used as supplied: no implicit
/// root, environment substitution, merging, watcher or reload timer. Batch
/// preparation pins file bytes and publishes one atomic owner-keyed catalog.
pub struct ConfigReader {
    parsers: HashMap<String, Arc<dyn ConfigParser>>,
    configs: ConfigStore<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
    preparation: tokio::sync::Mutex<()>,
    sources: Option<tokio::sync::Mutex<HashMap<PathBuf, Arc<[u8]>>>>,
}

impl Default for ConfigReader {
    fn default() -> Self {
        Self {
            parsers: HashMap::new(),
            configs: ConfigStore::new(HashMap::new()),
            preparation: tokio::sync::Mutex::new(()),
            sources: None,
        }
    }
}

impl ConfigReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepare every affected owner against one file-byte generation. A failed
    /// read, parse or validation leaves the entire published catalog unchanged.
    /// Runtime resource changes are deliberately outside this transaction.
    pub async fn prepare_owners(
        &self,
        owners: &[&dyn ConfigOwner],
    ) -> Result<()> {
        let _guard = self.preparation.lock().await;
        let reader = Self {
            parsers: self.parsers.clone(),
            sources: Some(tokio::sync::Mutex::new(HashMap::new())),
            configs: ConfigStore::new(self.configs.snapshot().as_ref().clone()),
            preparation: tokio::sync::Mutex::new(()),
        };
        let mut candidate = self.configs.snapshot().as_ref().clone();
        for owner in owners {
            candidate.insert(
                (*owner).type_id(),
                owner.prepare_config(&reader).await?,
            );
            // Later owners may resolve already-prepared dependency configs.
            reader.configs.replace(candidate.clone(), |_| Ok(()))?;
        }
        self.configs.replace(candidate, |_| Ok(()))?;
        Ok(())
    }

    /// Pin the prepared config under its owner type, not under its schema type.
    pub fn snapshot<C: Configurable + 'static>(
        &self
    ) -> Result<Arc<C::Config>> {
        self.configs
            .snapshot()
            .get(&TypeId::of::<C>())
            .cloned()
            .and_then(|value| value.downcast::<C::Config>().ok())
            .ok_or_else(|| Error::MissingConfig(std::any::type_name::<C>()))
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
    /// replace an active snapshot or boot/reload any runtime resources.
    pub async fn prepare<C: Configurable + ?Sized>(
        &self,
        owner: &C,
    ) -> Result<C::Config> {
        let candidate = owner.configure(self).await?;
        owner.validate_config(&candidate)?;
        Ok(candidate)
    }

    pub fn register<P: ConfigParser + 'static>(
        &mut self,
        extension: &str,
        parser: P,
    ) -> Result<&mut Self> {
        if extension.is_empty()
            || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(Error::InvalidExtension(extension.to_owned()));
        }
        let extension = extension.to_ascii_lowercase();
        if self.parsers.contains_key(&extension) {
            return Err(Error::DuplicateParser(extension));
        }
        self.parsers.insert(extension, Arc::new(parser));
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

    pub async fn read<T: DeserializeOwned>(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<T> {
        let path = path.as_ref();
        // Resolve before I/O: an unsupported format is not opened/executed.
        let parser = self.parser(path)?;
        let bytes = self.read_bytes(path).await?;
        Self::decode(parser, ConfigSource { path, bytes: &bytes }, None).await
    }

    /// Read a file, then expand caller-supplied variables in parsed strings.
    pub async fn read_with_variables<T: DeserializeOwned>(
        &self,
        path: impl AsRef<Path>,
        variables: &Variables,
    ) -> Result<T> {
        let path = path.as_ref();
        let parser = self.parser(path)?;
        let bytes = self.read_bytes(path).await?;
        Self::decode(
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
        Self::decode(self.parser(source.path)?, source, None).await
    }

    /// Parse first, expand `${NAME}` in values, then apply the provider schema.
    /// No environment lookup, recursive expansion or implicit type coercion.
    pub async fn parse_with_variables<T: DeserializeOwned>(
        &self,
        source: ConfigSource<'_>,
        variables: &Variables,
    ) -> Result<T> {
        Self::decode(self.parser(source.path)?, source, Some(variables)).await
    }

    async fn decode<T: DeserializeOwned>(
        parser: &dyn ConfigParser,
        source: ConfigSource<'_>,
        variables: Option<&Variables>,
    ) -> Result<T> {
        let mut value = parser.parse(source).await.map_err(|error| {
            Error::Parse { path: source.path.to_owned(), source: error }
        })?;
        if let Some(variables) = variables {
            crate::variables::resolve(&mut value, variables)?;
        }
        T::deserialize(value).map_err(|error| Error::Deserialize {
            path: source.path.to_owned(),
            source: error,
        })
    }
}
