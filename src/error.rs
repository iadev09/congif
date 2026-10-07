use std::path::PathBuf;

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("prepared configuration belongs to another reader")]
    ForeignPreparedConfig,
    #[error(
        "invalid configuration key '{0}' (expected nonempty dot-separated components)"
    )]
    InvalidKey(String),
    #[error(
        "invalid configuration namespace '{0}' (expected a nonempty name without dots)"
    )]
    InvalidNamespace(String),
    #[error("configuration namespace '{0}' is already owned")]
    DuplicateNamespace(String),
    #[error(
        "configuration key '{key}' cannot be converted to {expected}: {source}"
    )]
    Lookup {
        key: String,
        expected: &'static str,
        source: serde_value::DeserializerError,
    },
    #[error("configuration environment failed: {0}")]
    Environment(#[from] dotenw::Error),
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
    #[error("config parser must declare at least one file extension")]
    MissingExtensions,
    #[error("config variable '{0}' is not defined")]
    UndefinedVariable(String),
    #[error(
        "unquoted JSON variable '{0}' must contain a number, boolean or null"
    )]
    InvalidVariableValue(String),
    #[error("invalid config variable expression (expected $NAME or ${{NAME}})")]
    InvalidVariable,
    #[error("config validation failed: {0}")]
    Validation(#[source] BoxError),
    #[error("{0}")]
    Message(String),
    #[cfg(feature = "watch")]
    #[error("config watch failed: {0}")]
    Watch(#[source] BoxError),
}

impl Error {
    /// A validation failure carrying an error value of its own.
    pub fn validation(
        error: impl std::error::Error + Send + Sync + 'static
    ) -> Self {
        Self::Validation(Box::new(error))
    }

    /// A validation failure that is only a sentence: "default store is not
    /// configured". Reads as a validation error wherever errors are matched.
    pub fn invalid(message: impl std::fmt::Display) -> Self {
        Self::Validation(Box::new(Message(message.to_string())))
    }

    /// A failure outside reading and validation — an apply step that refused,
    /// a poisoned lock, an owner that was never prepared.
    pub fn msg(message: impl std::fmt::Display) -> Self {
        Self::Message(message.to_string())
    }
}

#[derive(Debug)]
struct Message(String);

impl std::fmt::Display for Message {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Message {}
