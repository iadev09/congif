use std::ops::Deref;
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::Result;

/// A pinned configuration generation. Clone shares the same immutable value.
pub struct Snapshot<T>(Arc<T>);

impl<T> Clone for Snapshot<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Deref for Snapshot<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> AsRef<T> for Snapshot<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

/// One atomic snapshot owner. `T` may contain all an application's provider
/// configs. Pin one snapshot per operation to avoid mixing generations.
///
/// Configs should be immutable values; interior mutability can defeat snapshot
/// isolation. This stores configuration, not sockets, pools or running tasks.
pub struct ConfigStore<T> {
    current: ArcSwap<T>,
}

impl<T> ConfigStore<T> {
    /// The caller supplies a fully prepared initial configuration.
    pub fn new(config: T) -> Self {
        Self { current: ArcSwap::from_pointee(config) }
    }

    pub fn snapshot(&self) -> Snapshot<T> {
        Snapshot(self.current.load_full())
    }

    /// Validate the complete candidate before publishing it. Failed validation
    /// leaves the active generation unchanged. Returns the previous snapshot.
    /// The validator is provider/application-owned; this store interprets no
    /// domain rules. Concurrent successful writers publish in swap order.
    pub fn replace(
        &self,
        candidate: T,
        validate: impl FnOnce(&T) -> Result<()>,
    ) -> Result<Snapshot<T>> {
        validate(&candidate)?;
        Ok(Snapshot(self.current.swap(Arc::new(candidate))))
    }
}
