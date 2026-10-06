//! Built-in adapters are feature-gated; the custom parser contract is always available.

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
