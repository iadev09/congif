use std::any::TypeId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use congif::{
    ConfigFile, ConfigReader, Configurable, ProgramEnvironment, Result,
    async_trait,
};
use serde::Deserialize;

#[derive(Deserialize, PartialEq)]
struct Settings {
    value: u64,
}

/// An owner that runs one program per `value`; the fixture is only here to
/// give it a source.
struct Owner(PathBuf);

#[async_trait]
impl Configurable for Owner {
    type Config = Settings;

    fn source(&self) -> ConfigFile {
        ConfigFile::yaml(self.0.clone())
    }

    async fn configure(
        &self,
        _: &Settings,
    ) -> Result<()> {
        Ok(())
    }
}

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "congif-programs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("settings.yaml"), "value: 1\n").unwrap();
        Self(directory)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn a_program_holds_only_its_own_keys_and_follows_its_files() {
    let directory = Directory::new();
    let app = directory.0.join("app.env");
    std::fs::write(&app, "APP_ROOT=${ROOT}/app\n").unwrap();
    let store = dotenw::EnvStore::from_base(dotenw::Snapshot::new(
        dotenw::Values::from([
            ("ROOT".to_owned(), "/srv/example".to_owned()),
            ("HOST_SECRET".to_owned(), "hidden".to_owned()),
        ]),
    ))
    .unwrap();
    let reader = ConfigReader::new()
        .with_environment_store(store)
        .with_builtin_parsers()
        .unwrap();
    let owner = Owner(directory.0.join("settings.yaml"));
    reader.initialize_owners(&[&owner]).await.unwrap();
    let id = TypeId::of::<Owner>();
    reader
        .prepare_programs(
            id,
            vec![
                ProgramEnvironment::new("web")
                    .file(&app)
                    .values([("APP_PROFILE".to_owned(), "web".to_owned())]),
            ],
        )
        .unwrap();
    let snapshot = reader.program_snapshot(id, "web").unwrap();
    assert_eq!(
        snapshot.get("APP_ROOT").map(String::as_str),
        Some("/srv/example/app")
    );
    assert_eq!(snapshot.get("APP_PROFILE").map(String::as_str), Some("web"));
    assert!(!snapshot.contains_key("HOST_SECRET"));
    assert!(reader.program_snapshot(id, "cli").is_err());
    // The absent `.production` variant is registered too, so it is watched.
    let sources = reader.program_sources(id).unwrap();
    let canonical = std::fs::canonicalize(&directory.0).unwrap();
    assert!(sources.contains(&canonical.join("app.env")));
    assert!(sources.contains(&canonical.join("app.env.production")));

    // A changed file is re-read by the same notification path as the
    // configuration, and the owner is told which program it was.
    std::fs::write(&app, "APP_ROOT=${ROOT}/next\n").unwrap();
    reader
        .prepare_changed_sources(&[&owner], &[canonical.join("app.env")])
        .await
        .unwrap();
    let changes = reader.take_program_changes().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].0, id);
    assert_eq!(changes[0].1, "web");
    assert_eq!(
        changes[0].2.get("APP_ROOT").map(String::as_str),
        Some("/srv/example/next")
    );
    assert!(reader.take_program_changes().unwrap().is_empty());

    // Re-declaring replaces the recipe on the same node.
    reader
        .prepare_programs(
            id,
            vec![
                ProgramEnvironment::new("web")
                    .values([("APP_PROFILE".to_owned(), "console".to_owned())]),
            ],
        )
        .unwrap();
    let snapshot = reader.program_snapshot(id, "web").unwrap();
    assert_eq!(
        snapshot.get("APP_PROFILE").map(String::as_str),
        Some("console")
    );
    assert!(!snapshot.contains_key("APP_ROOT"));
}
