//! Program environments: what a provider starts its programs with.
//!
//! A configuration owner may run programs — a PHP application or script, a
//! task's process — each with an environment of its own: `env_file` entries
//! and inline values declared in the owner's configuration. The reader
//! registers one dotenw node per program, layered like its own `.env`
//! (`<file>` then `<file>.<environment>`), detached from the host's values
//! (only what the files and values declare is in the result, while `${NAME}`
//! expands from the owner's configuration environment) and watched by the
//! same watcher as the configuration. Reading a changed file is the reader's;
//! what follows is the owner's.

use std::path::PathBuf;

/// One program's declaration: its files, as the owner resolved them, and the
/// values written beside them. Later files override earlier ones; values
/// override every file.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramEnvironment {
    pub name: String,
    pub files: Vec<PathBuf>,
    pub values: dotenw::Values,
}

impl ProgramEnvironment {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            files: Vec::new(),
            values: dotenw::Values::new(),
        }
    }

    pub fn file(
        mut self,
        path: impl Into<PathBuf>,
    ) -> Self {
        self.files.push(path.into());
        self
    }

    pub fn files(
        mut self,
        paths: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> Self {
        self.files.extend(paths.into_iter().map(Into::into));
        self
    }

    pub fn values(
        mut self,
        values: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        self.values.extend(values);
        self
    }
}

/// A provider whose programs are started with environments of their own.
///
/// Declared from the applied configuration, so the runtime asks after
/// `configure` and again after a reload; the reader keeps one node per
/// program name and replaces its recipe when the declaration changed. When
/// a program's file is re-read the runtime calls `environment_changed`: the
/// provider decides what follows — the next instance starts with it, the
/// running ones are rotated, or nothing until a restart.
pub trait Environmental: Send + Sync {
    fn program_environments(&self) -> Vec<ProgramEnvironment>;

    fn environment_changed(
        &self,
        program: &str,
        snapshot: &dotenw::Snapshot,
    );
}
