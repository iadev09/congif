use std::collections::BTreeMap;

use crate::{BoxError, ConfigParser, ConfigSource, ConfigValue, async_trait};

/// INI sections become maps; unnamed properties live at the root.
/// Values remain strings, names are case-sensitive, duplicate keys are errors.
pub struct IniParser;

#[async_trait]
impl ConfigParser for IniParser {
    fn extensions(&self) -> &[&str] {
        &["ini"]
    }

    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        let document = ::ini::Ini::load_from_str_noescape(
            std::str::from_utf8(source.bytes)?,
        )?;
        let mut root = BTreeMap::new();
        for (section, properties) in &document {
            let mut entries = BTreeMap::new();
            for (name, value) in properties {
                if entries
                    .insert(
                        ConfigValue::String(name.into()),
                        ConfigValue::String(value.into()),
                    )
                    .is_some()
                {
                    return Err(std::io::Error::other(
                        "duplicate INI property",
                    )
                    .into());
                }
            }
            if let Some(name) = section {
                if root
                    .insert(
                        ConfigValue::String(name.into()),
                        ConfigValue::Map(entries),
                    )
                    .is_some()
                {
                    return Err(std::io::Error::other(
                        "duplicate INI section or root property",
                    )
                    .into());
                }
            } else {
                for (name, value) in entries {
                    if root.insert(name, value).is_some() {
                        return Err(std::io::Error::other(
                            "duplicate INI root property",
                        )
                        .into());
                    }
                }
            }
        }
        Ok(ConfigValue::Map(root))
    }
}
