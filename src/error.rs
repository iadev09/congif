use std::path::PathBuf;

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration for owner '{0}' has not been prepared")]
    MissingConfig(&'static str),
    #[error("could not read config {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("could not parse config {path}: {source}")]
    Parse { path: PathBuf, source: BoxError },
    #[error("config {path} does not match its schema: {source}")]
    Deserialize { path: PathBuf, source: serde_value::DeserializerError },
    #[error("no config parser registered for {path}")]
    UnsupportedFormat { path: PathBuf },
    #[error("config parser already registered for '{0}'")]
    DuplicateParser(String),
    #[error("invalid config extension '{0}' (use a bare ASCII extension)")]
    InvalidExtension(String),
    #[error("config variable '{0}' is not defined")]
    UndefinedVariable(String),
    #[error("invalid config variable expression (expected ${{NAME}})")]
    InvalidVariable,
    #[error("config validation failed: {0}")]
    Validation(#[source] BoxError),
    #[cfg(feature = "watch")]
    #[error("config watch failed: {0}")]
    Watch(#[source] BoxError),
}

impl Error {
    pub fn validation(
        error: impl std::error::Error + Send + Sync + 'static
    ) -> Self {
        Self::Validation(Box::new(error))
    }
}
