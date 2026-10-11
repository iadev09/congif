use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

struct AppState(ConfigReader);

impl congif::HasConfigReader for AppState {
    fn config_reader(&self) -> &ConfigReader {
        &self.0
    }
}

use congif::ConfigValue;
use congif::{ConfigReader, Configurable, Error, Result, async_trait};
use std::collections::BTreeMap;

struct Owner<const N: u8> {
    value: Arc<AtomicU64>,
    preparations: AtomicU64,
}

struct Exposed<const N: u8>(Arc<AtomicU64>);

struct AtomicParser {
    key: [&'static str; 1],
    value: Arc<AtomicU64>,
}

#[async_trait]
impl congif::ConfigParser for AtomicParser {
    fn extensions(&self) -> &[&str] {
        &self.key
    }
    async fn parse(
        &self,
        _: congif::ConfigSource<'_>,
    ) -> std::result::Result<ConfigValue, congif::BoxError> {
        Ok(ConfigValue::U64(self.value.load(Ordering::Relaxed)))
    }
}

fn source<const N: u8>() -> congif::ConfigFile {
    congif::ConfigFile {
        path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/batch/first.json"),
        format: congif::Format::Custom(format!("v{N}")),
    }
}

#[async_trait]
impl<const N: u8> Configurable for Exposed<N> {
    type Config = u64;
    fn namespace(&self) -> String {
        "http".to_owned()
    }
    fn source(&self) -> congif::ConfigFile {
        source::<N>()
    }
    async fn configure(
        &self,
        _: &u64,
    ) -> Result<()> {
        Ok(())
    }
    fn export_config(
        &self,
        value: &u64,
    ) -> Result<Option<ConfigValue>> {
        Ok(Some(ConfigValue::Map(BTreeMap::from([
            (
                ConfigValue::String("listen".to_owned()),
                ConfigValue::String("127.0.0.1:8080".to_owned()),
            ),
            (
                ConfigValue::String("nested".to_owned()),
                ConfigValue::Map(BTreeMap::from([(
                    ConfigValue::String("values".to_owned()),
                    ConfigValue::Seq(vec![ConfigValue::U64(*value)]),
                )])),
            ),
        ]))))
    }
}

#[tokio::test]
async fn prepared_and_applied_generations_only_converge_after_confirmation() {
    let reader = ConfigReader::new();
    let owner = Exposed::<0>(Arc::new(AtomicU64::new(1)));
    reader
        .register(AtomicParser { key: ["v0"], value: owner.0.clone() })
        .unwrap();
    let owners: [&dyn congif::ConfigRegistration; 1] = [&owner];
    let id = std::any::TypeId::of::<Exposed<0>>();
    reader.initialize_owners(&owners).await.unwrap();
    assert!(matches!(
        ConfigReader::new()
            .confirm_applied(reader.prepared_config(&owner).unwrap())
            .await,
        Err(Error::ForeignPreparedConfig)
    ));
    reader
        .confirm_applied(reader.prepared_config(&owner).unwrap())
        .await
        .unwrap();
    assert!(reader.prepare_changed_owners(&owners).await.unwrap().is_empty());

    owner.0.store(2, Ordering::Relaxed);
    assert_eq!(reader.prepare_changed_owners(&owners).await.unwrap(), vec![id]);
    let second = reader.prepared_config(&owner).unwrap();
    // A failed resource update leaves 1 applied, so an identical candidate
    // still needs application on the next notification.
    assert_eq!(reader.prepare_changed_owners(&owners).await.unwrap(), vec![id]);

    owner.0.store(3, Ordering::Relaxed);
    reader.prepare_changed_owners(&owners).await.unwrap();
    reader.confirm_applied(second).await.unwrap();
    // Confirming 2 cannot acknowledge the newer prepared value 3.
    assert_eq!(reader.prepare_changed_owners(&owners).await.unwrap(), vec![id]);
    reader
        .confirm_applied(reader.prepared_config(&owner).unwrap())
        .await
        .unwrap();
    assert!(reader.prepare_changed_owners(&owners).await.unwrap().is_empty());

    owner.0.store(4, Ordering::Relaxed);
    reader.prepare_changed_owners(&owners).await.unwrap();
    // Revert a rejected candidate to the actual applied config: publish the
    // prepared correction, but no resource update is required.
    owner.0.store(3, Ordering::Relaxed);
    assert!(reader.prepare_changed_owners(&owners).await.unwrap().is_empty());
    assert_eq!(*reader.snapshot::<Exposed<0>>().unwrap(), 3);
}

#[tokio::test]
async fn dot_lookup_distinguishes_missing_values_and_conversion_errors() {
    let reader = ConfigReader::new();
    let owner = Exposed::<0>(Arc::new(AtomicU64::new(3)));
    reader
        .register(AtomicParser { key: ["v0"], value: owner.0.clone() })
        .unwrap();
    reader.initialize_owners(&[&owner]).await.unwrap();
    let address =
        reader.config::<std::net::SocketAddr>("http.listen").unwrap().unwrap();
    assert_eq!(address.port(), 8080);
    use congif::HasConfigReader;
    let state = AppState(reader);
    assert_eq!(
        state.get_config::<std::net::SocketAddr>("http.listen").unwrap(),
        Some(address)
    );
    let reader = state.config_reader();
    assert_eq!(
        reader.config::<std::net::SocketAddr>("http.listen").unwrap(),
        Some(address)
    );
    assert_eq!(reader.config::<u64>("http.nested.values.0").unwrap(), Some(3));
    assert_eq!(reader.config::<u64>("http.missing").unwrap(), None);
    assert_eq!(reader.config::<u64>("unknown.value").unwrap(), None);
    assert_eq!(reader.config::<u64>("http.nested.values.1").unwrap(), None);
    assert!(
        matches!(reader.config::<u64>("http.listen"), Err(Error::Lookup { key, .. }) if key == "http.listen")
    );
    assert!(matches!(reader.value("http..listen"), Err(Error::InvalidKey(_))));
    owner.0.store(4, Ordering::Relaxed);
    reader.prepare_owners(&[&owner]).await.unwrap();
    assert_eq!(reader.config::<u64>("http.nested.values.0").unwrap(), Some(4));
    assert_eq!(*reader.snapshot::<Exposed<0>>().unwrap(), 4);
}

#[tokio::test]
async fn namespace_conflict_keeps_the_previous_catalog() {
    let reader = ConfigReader::new();
    let first = Exposed::<0>(Arc::new(AtomicU64::new(1)));
    reader
        .register(AtomicParser { key: ["v0"], value: first.0.clone() })
        .unwrap();
    reader.initialize_owners(&[&first]).await.unwrap();
    first.0.store(2, Ordering::Relaxed);
    let second = Exposed::<1>(Arc::new(AtomicU64::new(3)));
    reader
        .register(AtomicParser { key: ["v1"], value: second.0.clone() })
        .unwrap();
    assert!(matches!(
        reader.prepare_owners(&[&first, &second]).await,
        Err(Error::DuplicateNamespace(_))
    ));
    assert_eq!(*reader.snapshot::<Exposed<0>>().unwrap(), 1);
    assert_eq!(reader.config::<u64>("http.nested.values.0").unwrap(), Some(1));
}

#[async_trait]
impl<const N: u8> Configurable for Owner<N> {
    type Config = u64;

    fn source(&self) -> congif::ConfigFile {
        source::<N>()
    }
    async fn configure(
        &self,
        _: &u64,
    ) -> Result<()> {
        Ok(())
    }
    fn validate_config(
        &self,
        _: &u64,
    ) -> Result<()> {
        self.preparations.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[tokio::test]
async fn access_shares_the_prepared_snapshot_and_pins_its_generation() {
    let reader = ConfigReader::new();
    let owner = Owner::<0> {
        value: Arc::new(AtomicU64::new(1)),
        preparations: AtomicU64::new(0),
    };
    reader
        .register(AtomicParser { key: ["v0"], value: owner.value.clone() })
        .unwrap();
    reader.initialize_owners(&[&owner]).await.unwrap();
    let pinned = reader.snapshot::<Owner<0>>().unwrap();
    assert!(Arc::ptr_eq(&pinned, &reader.snapshot::<Owner<0>>().unwrap()));
    assert_eq!(owner.preparations.load(Ordering::Relaxed), 1);

    owner.value.store(2, Ordering::Relaxed);
    reader.prepare_owners(&[&owner]).await.unwrap();
    assert_eq!(*reader.snapshot::<Owner<0>>().unwrap(), 2);
    assert_eq!(*pinned, 1);
    assert_eq!(owner.preparations.load(Ordering::Relaxed), 2);
    // The same schema under an unprepared owner is not another owner's config.
    assert!(matches!(
        reader.snapshot::<Owner<1>>(),
        Err(Error::MissingConfig(_))
    ));
}
