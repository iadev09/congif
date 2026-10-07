use std::path::PathBuf;

/// Resolve a glob against the files present for this read. It is not a
/// directory subscription; later additions appear only on another source read.
pub(crate) async fn resolve_includes(
    source: &std::path::Path,
    includes: Vec<PathBuf>,
) -> crate::Result<Vec<PathBuf>> {
    if includes.is_empty() {
        return Ok(Vec::new());
    }
    let source = source.to_owned();
    let error_path = source.clone();
    tokio::task::spawn_blocking(move || {
        let parent = source.parent().unwrap_or(std::path::Path::new("."));
        let mut paths = Vec::new();
        for include in includes {
            let path = if include.is_absolute() {
                include
            } else {
                parent.join(include)
            };
            if let Some(pattern) =
                path.to_str().filter(|path| path.contains(['*', '?', '[']))
            {
                let matches = glob::glob(pattern).map_err(|error| {
                    crate::Error::Parse {
                        path: source.clone(),
                        source: Box::new(error),
                    }
                })?;
                for matched in matches {
                    let path =
                        matched.map_err(|error| crate::Error::Parse {
                            path: source.clone(),
                            source: Box::new(error),
                        })?;
                    match std::fs::metadata(&path) {
                        Ok(metadata) if metadata.is_file() => paths.push(path),
                        Ok(_) => {}
                        Err(error)
                            if error.kind() == std::io::ErrorKind::NotFound => {
                        }
                        Err(error) => {
                            return Err(crate::Error::Read {
                                path,
                                source: error,
                            });
                        }
                    }
                }
            } else {
                // A named missing file is known and can be corrected/created.
                paths.push(path);
            }
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    })
    .await
    .map_err(|error| crate::Error::Parse {
        path: error_path,
        source: Box::new(error),
    })?
}

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

impl ConfigFile {
    pub fn new(
        path: impl Into<PathBuf>,
        format: Format,
    ) -> Self {
        Self { path: path.into(), format }
    }

    pub fn json(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Format::Json)
    }

    pub fn yaml(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Format::Yaml)
    }

    pub fn toml(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Format::Toml)
    }

    pub fn ini(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Format::Ini)
    }

    pub fn xml(path: impl Into<PathBuf>) -> Self {
        Self::new(path, Format::Xml)
    }
}
