//! Async, extensible configuration reading and atomic snapshot ownership.
//!
//! Providers own schemas, custom deserializers, validation and compilation.
//! This crate owns neither their lifecycle nor their runtime resources.
#![forbid(unsafe_code)]

mod configurable;
mod error;
pub mod parsers;
mod reader;
mod source;
mod state;
mod store;
mod variables;
#[cfg(all(
    feature = "watch",
    any(target_os = "linux", target_os = "macos", target_os = "freebsd")
))]
pub mod watch;

#[cfg(all(
    feature = "watch",
    not(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))
))]
compile_error!(
    "congif's `watch` feature supports Linux, macOS and FreeBSD only; disable `watch` on this target (no polling fallback)"
);

pub use async_trait::async_trait;
pub use configurable::{ConfigObserver, ConfigRegistration, Configurable};
pub use error::{BoxError, Error, Result};
pub use parsers::{ConfigParser, ConfigSource};
pub use reader::{ConfigReader, PreparedConfig};
pub use serde_value::Value as ConfigValue;
pub use source::{ConfigFile, Format};
pub use state::HasConfigReader;
pub use store::{ConfigStore, Snapshot};
pub use variables::Variables;
