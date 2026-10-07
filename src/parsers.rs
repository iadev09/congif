//! Built-in adapters are feature-gated; the custom parser contract is always available.

use std::collections::HashMap;
use std::sync::Arc;

mod custom;
pub use custom::{ConfigParser, ConfigSource};

#[cfg(feature = "ini")]
mod ini;
#[cfg(feature = "ini")]
pub use ini::IniParser;
#[cfg(feature = "xml")]
mod xml;
#[cfg(feature = "xml")]
pub use xml::XmlParser;
#[cfg(feature = "yaml")]
mod yaml;
#[cfg(feature = "yaml")]
pub use yaml::YamlParser;
#[cfg(feature = "toml")]
mod toml;
#[cfg(feature = "toml")]
pub use toml::TomlParser;
#[cfg(feature = "json")]
mod json;
#[cfg(feature = "json")]
pub use json::JsonParser;

pub(crate) fn builtin_parsers() -> HashMap<String, Arc<dyn ConfigParser>> {
    let builtins: &[Arc<dyn ConfigParser>] = &[
        #[cfg(feature = "json")]
        Arc::new(JsonParser),
        #[cfg(feature = "yaml")]
        Arc::new(YamlParser),
        #[cfg(feature = "toml")]
        Arc::new(TomlParser),
        #[cfg(feature = "ini")]
        Arc::new(IniParser),
        #[cfg(feature = "xml")]
        Arc::new(XmlParser),
    ];
    let mut parsers = HashMap::new();
    for parser in builtins {
        for extension in parser.extensions() {
            parsers.insert(extension.to_ascii_lowercase(), parser.clone());
        }
    }
    parsers
}
