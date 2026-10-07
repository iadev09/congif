use std::path::PathBuf;

use crate::{BoxError, ConfigValue};

/// `includes` is reader metadata wherever it occurs in a YAML/TOML value tree.
/// Consume metadata before the provider's schema sees the value tree.
pub(super) fn paths(value: &mut ConfigValue) -> Result<Vec<PathBuf>, BoxError> {
    let mut paths = Vec::new();
    collect(value, &mut paths)?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn collect(
    value: &mut ConfigValue,
    paths: &mut Vec<PathBuf>,
) -> Result<(), BoxError> {
    match value {
        ConfigValue::Map(map) => {
            for key in ["includes", "include"] {
                if let Some(value) =
                    map.remove(&ConfigValue::String(key.to_owned()))
                {
                    let included: Vec<PathBuf> =
                        if matches!(value, ConfigValue::Seq(_)) {
                            value.deserialize_into().map_err(|error| {
                                std::io::Error::other(format!(
                                    "invalid `{key}` paths: {error}"
                                ))
                            })?
                        } else {
                            vec![value.deserialize_into().map_err(|error| {
                                std::io::Error::other(format!(
                                    "invalid `{key}` path: {error}"
                                ))
                            })?]
                        };
                    if included.iter().any(|path| path.as_os_str().is_empty()) {
                        return Err(std::io::Error::other(
                            "`includes` requires nonempty file paths",
                        )
                        .into());
                    }
                    paths.extend(included);
                }
            }
            for value in map.values_mut() {
                collect(value, paths)?;
            }
        }
        ConfigValue::Seq(values) => {
            for value in values {
                collect(value, paths)?;
            }
        }
        _ => {}
    }
    Ok(())
}
