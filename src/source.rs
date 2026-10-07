use std::path::PathBuf;

/// Explicit adapter selection, independent of the filename extension.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Json,
    Yaml,
    Toml,
    Ini,
    Xml,
    Custom(String),
}

impl Format {
    pub(crate) fn key(&self) -> &str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Ini => "ini",
            Self::Xml => "xml",
            Self::Custom(name) => name,
        }
    }
}

/// One typed configuration input. Watch capability is declared separately.
#[derive(Clone, Debug)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub format: Format,
}
