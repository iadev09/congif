#![cfg(feature = "json")]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use congif::{
    ConfigObserver, ConfigReader, ConfigRegistration, Configurable, Result,
    async_trait,
};
use dotenw::{EnvHandle, EnvStore, Environment, MergeMode, Values};
use serde::Deserialize;

#[derive(Deserialize, PartialEq)]
struct Settings {
    value: String,
}

struct Owner<const N: u8> {
    environment: EnvHandle,
    notifications: AtomicUsize,
    configurations: AtomicUsize,
    reject: AtomicBool,
}

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/environment/settings.json")
}

#[async_trait]
impl<const N: u8> Configurable for Owner<N> {
    type Config = Settings;
    fn source(&self) -> congif::ConfigFile {
        congif::ConfigFile { path: path(), format: congif::Format::Json }
    }
    fn environment(&self) -> Option<&EnvHandle> {
        Some(&self.environment)
    }
    fn as_config_observer(
        &self
    ) -> Option<&dyn ConfigObserver<Config = Settings>> {
        Some(self)
    }
    async fn configure(
        &self,
        config: &Settings,
    ) -> Result<()> {
        if self.reject.load(Ordering::Relaxed) {
            return Err(congif::Error::invalid("startup rejected"));
        }
        assert_eq!(
            config.value,
            self.environment.snapshot()?.get("VALUE").unwrap().as_str()
        );
        self.configurations.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[async_trait]
impl<const N: u8> ConfigObserver for Owner<N> {
    async fn config_changed(
        &self,
        config: &Settings,
    ) -> Result<()> {
        if self.reject.load(Ordering::Relaxed) {
            return Err(congif::Error::invalid("application rejected"));
        }
        assert_eq!(
            config.value,
            self.environment.snapshot()?.get("VALUE").unwrap().as_str()
        );
        self.notifications.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn owner<const N: u8>(
    store: &EnvStore,
    value: &str,
) -> Owner<N> {
    Owner {
        environment: store
            .register(Environment::new().with_values(
                Values::from([("VALUE".into(), value.into())]),
                MergeMode::Override,
            ))
            .unwrap(),
        notifications: AtomicUsize::new(0),
        configurations: AtomicUsize::new(0),
        reject: AtomicBool::new(false),
    }
}

#[tokio::test]
async fn preparation_does_not_notify_and_observers_receive_their_own_environment()
 {
    let store = EnvStore::new();
    let first = owner::<1>(&store, "first");
    let second = owner::<2>(&store, "second");
    let reader = ConfigReader::new()
        .with_environment_store(store)
        .with_builtin_parsers()
        .unwrap();
    let owners: [&dyn ConfigRegistration; 2] = [&first, &second];
    assert_eq!(reader.prepare(&first).await.unwrap().value, "first");
    reader.initialize_owners(&owners).await.unwrap();
    assert_eq!(reader.snapshot::<Owner<1>>().unwrap().value, "first");
    assert_eq!(reader.snapshot::<Owner<2>>().unwrap().value, "second");
    assert_eq!(first.notifications.load(Ordering::Relaxed), 0);
    assert_eq!(first.configurations.load(Ordering::Relaxed), 0);
    reader.notify_changed(&first).await.unwrap();
    reader.notify_changed(&second).await.unwrap();
    assert_eq!(first.notifications.load(Ordering::Relaxed), 1);
    assert_eq!(second.notifications.load(Ordering::Relaxed), 1);
    let previous = reader.snapshot::<Owner<1>>().unwrap();
    assert!(reader.prepare_changed_owners(&owners).await.unwrap().is_empty());
    assert!(Arc::ptr_eq(&previous, &reader.snapshot::<Owner<1>>().unwrap()));
}

#[tokio::test]
async fn startup_receives_prepared_config_and_acknowledges_only_success() {
    let store = EnvStore::new();
    let provider = owner::<5>(&store, "startup");
    let reader = ConfigReader::new()
        .with_environment_store(store)
        .with_builtin_parsers()
        .unwrap();
    reader.initialize_owners(&[&provider]).await.unwrap();
    assert_eq!(provider.configurations.load(Ordering::Relaxed), 0);
    provider.reject.store(true, Ordering::Relaxed);
    assert!(reader.configure_registered(&provider).await.is_err());
    assert_eq!(
        reader.prepare_changed_owners(&[&provider]).await.unwrap().len(),
        1
    );
    provider.reject.store(false, Ordering::Relaxed);
    reader.configure_registered(&provider).await.unwrap();
    assert_eq!(provider.configurations.load(Ordering::Relaxed), 1);
    assert_eq!(provider.notifications.load(Ordering::Relaxed), 0);
    assert!(
        reader.prepare_changed_owners(&[&provider]).await.unwrap().is_empty()
    );
}

#[tokio::test]
async fn failed_observer_does_not_acknowledge_the_new_candidate() {
    let store = EnvStore::new();
    let mut provider = owner::<3>(&store, "first");
    let reader = ConfigReader::new()
        .with_environment_store(store.clone())
        .with_builtin_parsers()
        .unwrap();
    reader.initialize_owners(&[&provider]).await.unwrap();
    reader.notify_changed(&provider).await.unwrap();
    provider.environment = owner::<4>(&store, "second").environment;
    reader.prepare_changed_owners(&[&provider]).await.unwrap();
    provider.reject.store(true, Ordering::Relaxed);
    assert!(reader.notify_changed(&provider).await.is_err());
    assert_eq!(
        reader.prepare_changed_owners(&[&provider]).await.unwrap(),
        vec![std::any::TypeId::of::<Owner<3>>()]
    );
    provider.reject.store(false, Ordering::Relaxed);
    reader.notify_changed(&provider).await.unwrap();
    assert!(
        reader.prepare_changed_owners(&[&provider]).await.unwrap().is_empty()
    );
}
