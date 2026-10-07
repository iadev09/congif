//! Optional native file notifications on Linux, macOS and FreeBSD only.
//! No config reading, lifecycle execution or polling fallback.

use std::any::TypeId;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use notify::{Event, EventKind, RecursiveMode, Watcher};

use crate::{Error, Result};

/// Notification addressed to the owner of a concrete config type.
/// Paths are absolute, with canonical parent directories.
pub struct ConfigChanged<C> {
    pub paths: Vec<PathBuf>,
    marker: PhantomData<fn() -> C>,
}

/// Source invalidation addressed to an owner's registration identity.
#[derive(Clone, Debug)]
pub struct SourceChanged {
    pub owner: TypeId,
    pub paths: Vec<PathBuf>,
}

type Handler = Arc<dyn Fn(Vec<PathBuf>) + Send + Sync>;

#[derive(Clone)]
struct Registration {
    owner: TypeId,
    paths: Vec<PathBuf>,
    handler: Handler,
}

/// One native watcher for all registered config sources: inotify on Linux,
/// FSEvents on macOS, and kqueue on FreeBSD. Other OS targets are unsupported.
///
/// Registrations last until this owner is dropped. Callbacks run on the native
/// notification thread: enqueue a lifecycle signal rather than blocking it.
/// Capture a weak provider reference if its lifetime is independently owned.
/// No debounce, retry cadence, automatic read, publication or reload is added.
pub struct ConfigWatcher {
    watcher: notify::RecommendedWatcher,
    registrations: Arc<RwLock<Vec<Registration>>>,
    directories: HashSet<PathBuf>,
}

impl ConfigWatcher {
    /// Backend errors (including lost events) are delivered to the caller.
    pub fn new(
        on_error: impl Fn(Error) + Send + Sync + 'static
    ) -> Result<Self> {
        let registrations = Arc::new(RwLock::new(Vec::new()));
        let targets = registrations.clone();
        let watcher =
            notify::recommended_watcher(move |event: notify::Result<Event>| {
                match event {
                    Ok(event) => {
                        let registrations = targets
                            .read()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .clone();
                        dispatch(&registrations, &event);
                    }
                    Err(error) => on_error(Error::Watch(Box::new(error))),
                }
            })
            .map_err(|error| Error::Watch(Box::new(error)))?;
        Ok(Self { watcher, registrations, directories: HashSet::new() })
    }

    /// Register one named-by-type config owner's files. Multiple owners of the
    /// same type are allowed; each has its own source list and callback.
    /// Parent directories must exist. File creation/removal and editor atomic
    /// replacement are observed by watching the parent, not the file inode.
    /// File symlinks are rejected; replacing watched parent directories requires
    /// the lifecycle owner to recreate the watcher.
    pub fn register<C: 'static>(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
        handler: impl Fn(ConfigChanged<C>) + Send + Sync + 'static,
    ) -> Result<&mut Self> {
        self.register_owner(TypeId::of::<C>(), paths, move |event| {
            handler(ConfigChanged { paths: event.paths, marker: PhantomData });
        })
    }

    /// Register an owner discovered dynamically by its application.
    /// This only reports invalidation; it does not read or reload the owner.
    pub fn register_owner(
        &mut self,
        owner: TypeId,
        paths: impl IntoIterator<Item = PathBuf>,
        handler: impl Fn(SourceChanged) + Send + Sync + 'static,
    ) -> Result<&mut Self> {
        let paths = normalized_paths(paths)?;
        self.watch_directories(&paths)?;
        tracing::debug!(owner = ?owner, paths = ?paths, directories = ?self.directories, "configuration sources watching registered");
        self.registrations
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Registration {
                owner,
                paths,
                handler: Arc::new(move |paths| {
                    handler(SourceChanged { owner, paths })
                }),
            });
        Ok(self)
    }

    /// Replace a registered owner's dependency list after successful reading.
    /// Keep its existing callback and watch new directories before dropping old
    /// dependencies. Multiple registrations under this identity are updated.
    pub fn update_owner_paths(
        &mut self,
        owner: TypeId,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Result<&mut Self> {
        let paths = normalized_paths(paths)?;
        let unchanged = {
            let registrations =
                self.registrations.read().unwrap_or_else(|p| p.into_inner());
            if !registrations
                .iter()
                .any(|registration| registration.owner == owner)
            {
                return Err(Error::msg(
                    "configuration watch owner is not registered",
                ));
            }
            registrations
                .iter()
                .filter(|registration| registration.owner == owner)
                .all(|registration| registration.paths == paths)
        };
        if unchanged {
            return Ok(self);
        }
        self.watch_directories(&paths)?;
        let required: HashSet<_> = {
            let mut registrations =
                self.registrations.write().unwrap_or_else(|p| p.into_inner());
            replace_owner_paths(&mut registrations, owner, &paths);
            registrations
                .iter()
                .flat_map(|registration| registration.paths.iter())
                .filter_map(|path| path.parent().map(Path::to_owned))
                .collect()
        };
        self.directories.retain(|directory| {
            if required.contains(directory) {
                return true;
            }
            if let Err(error) = self.watcher.unwatch(directory) {
                tracing::warn!(path = %directory.display(), %error, "unused configuration directory watch retained");
                return true;
            }
            false
        });
        tracing::debug!(owner = ?owner, paths = ?paths, "configuration watch dependencies updated");
        Ok(self)
    }

    fn watch_directories(
        &mut self,
        paths: &[PathBuf],
    ) -> Result<()> {
        let mut added: Vec<PathBuf> = Vec::new();
        for path in paths {
            let directory = path
                .parent()
                .expect("source_path returns an absolute file path");
            if self.directories.contains(directory)
                || added.iter().any(|p| p == directory)
            {
                continue;
            }
            if let Err(error) =
                self.watcher.watch(directory, RecursiveMode::NonRecursive)
            {
                // Failed registration installs no callback. Retain any watch
                // that cannot be rolled back so later registration can reuse it.
                for directory in added {
                    if self.watcher.unwatch(&directory).is_err() {
                        self.directories.insert(directory);
                    }
                }
                return Err(Error::Watch(Box::new(error)));
            }
            added.push(directory.to_owned());
        }
        self.directories.extend(added);
        Ok(())
    }

    /// Query source ownership from the existing registrations. An empty path
    /// list requests all owners (recovery after lost notifications).
    pub fn owners_for_paths(
        &self,
        paths: &[PathBuf],
    ) -> Vec<TypeId> {
        let registrations =
            self.registrations.read().unwrap_or_else(|p| p.into_inner());
        affected_owners(&registrations, paths)
    }
}

fn affected_owners(
    registrations: &[Registration],
    paths: &[PathBuf],
) -> Vec<TypeId> {
    let mut owners = Vec::new();
    for registration in registrations.iter() {
        if (paths.is_empty()
            || registration.paths.iter().any(|source| paths.contains(source)))
            && !owners.contains(&registration.owner)
        {
            owners.push(registration.owner);
        }
    }
    owners
}

fn normalized_paths(
    paths: impl IntoIterator<Item = PathBuf>
) -> Result<Vec<PathBuf>> {
    let mut paths: Vec<_> =
        paths.into_iter().map(source_path).collect::<Result<_>>()?;
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(Error::Watch(Box::new(std::io::Error::other(
            "config watch requires at least one source",
        ))));
    }
    Ok(paths)
}

fn replace_owner_paths(
    registrations: &mut [Registration],
    owner: TypeId,
    paths: &[PathBuf],
) {
    for registration in registrations
        .iter_mut()
        .filter(|registration| registration.owner == owner)
    {
        registration.paths = paths.to_vec();
    }
}

fn source_path(path: PathBuf) -> Result<PathBuf> {
    let map_error = |error| Error::Watch(Box::new(error) as crate::BoxError);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata)
            if metadata.file_type().is_symlink() || metadata.is_dir() =>
        {
            return Err(map_error(std::io::Error::other(
                "config watch source must be a file, not a directory or symlink",
            )));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(map_error(error));
        }
        _ => {}
    }
    let name = path.file_name().ok_or_else(|| {
        map_error(std::io::Error::other("config watch source has no filename"))
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(std::fs::canonicalize(parent).map_err(map_error)?.join(name))
}

fn dispatch(
    registrations: &[Registration],
    event: &Event,
) {
    // Opening/reading a config is not itself a config change.
    if matches!(event.kind, EventKind::Access(_)) && !event.need_rescan() {
        return;
    }
    let mut logged = false;
    for registration in registrations {
        let paths: Vec<_> = registration
            .paths
            .iter()
            .filter(|source| {
                event.need_rescan()
                    || event.paths.iter().any(|changed| {
                        source == &changed || source.starts_with(changed)
                    })
            })
            .cloned()
            .collect();
        if !paths.is_empty() {
            if !logged {
                tracing::trace!(kind = ?event.kind, paths = ?event.paths, "registered configuration source notification received");
                logged = true;
            }
            (registration.handler)(paths);
        }
    }
}

#[cfg(test)]
#[path = "../tests/watch/dispatch.rs"]
mod tests;
