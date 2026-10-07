use std::sync::Mutex;

use super::*;

fn target(
    path: &str,
    messages: Arc<Mutex<Vec<Vec<PathBuf>>>>,
) -> Registration {
    Registration {
        owner: TypeId::of::<()>(),
        paths: vec![PathBuf::from(path)],
        handler: Arc::new(move |paths| messages.lock().unwrap().push(paths)),
    }
}

#[test]
fn changes_only_notify_owners_of_matching_sources() {
    let first = Arc::new(Mutex::new(Vec::new()));
    let second = Arc::new(Mutex::new(Vec::new()));
    let targets = [
        target("/config/cache.yaml", first.clone()),
        target("/config/http.yaml", second.clone()),
    ];
    dispatch(
        &targets,
        &Event::new(EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        )))
        .add_path(PathBuf::from("/config/cache.yaml")),
    );
    assert_eq!(
        *first.lock().unwrap(),
        vec![vec![PathBuf::from("/config/cache.yaml")]]
    );
    assert!(second.lock().unwrap().is_empty());
}

#[test]
fn shared_file_selects_all_owners_without_duplicate_service_ids() {
    struct First;
    struct Second;
    struct Other;
    let messages = Arc::new(Mutex::new(Vec::new()));
    let mut first = target("/config/shared.json", messages.clone());
    first.owner = TypeId::of::<First>();
    let mut second = target("/config/shared.json", messages.clone());
    second.owner = TypeId::of::<Second>();
    let mut other = target("/config/other.json", messages);
    other.owner = TypeId::of::<Other>();
    let registrations = [first.clone(), second, other, first];
    assert_eq!(
        affected_owners(
            &registrations,
            &[PathBuf::from("/config/shared.json")]
        ),
        [TypeId::of::<First>(), TypeId::of::<Second>()]
    );
    assert_eq!(
        affected_owners(&registrations, &[]),
        [TypeId::of::<First>(), TypeId::of::<Second>(), TypeId::of::<Other>()]
    );
}

#[test]
fn shared_environment_selects_every_dependent_config_only_once() {
    struct Http;
    struct Counter;
    let messages = Arc::new(Mutex::new(Vec::new()));
    let mut http = target("/config/application.env", messages.clone());
    http.owner = TypeId::of::<Http>();
    http.paths.push(PathBuf::from("/config/http.json"));
    let mut counter = target("/config/application.env", messages);
    counter.owner = TypeId::of::<Counter>();
    counter.paths.push(PathBuf::from("/config/counter.json"));
    let registrations = [http, counter];
    assert_eq!(
        affected_owners(
            &registrations,
            &[
                PathBuf::from("/config/application.env"),
                PathBuf::from("/config/application.env"),
            ]
        ),
        [TypeId::of::<Http>(), TypeId::of::<Counter>()]
    );
    assert_eq!(
        affected_owners(&registrations, &[PathBuf::from("/config/http.json")]),
        [TypeId::of::<Http>()]
    );
}

#[test]
fn atomic_rename_notifies_destination_owner_but_reads_do_not() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let targets = [target("/config/cache.yaml", messages.clone())];
    dispatch(
        &targets,
        &Event::new(EventKind::Access(notify::event::AccessKind::Read))
            .add_path(PathBuf::from("/config/cache.yaml")),
    );
    dispatch(
        &targets,
        &Event::new(EventKind::Modify(notify::event::ModifyKind::Name(
            notify::event::RenameMode::Both,
        )))
        .add_path(PathBuf::from("/config/.temporary"))
        .add_path(PathBuf::from("/config/cache.yaml")),
    );
    assert_eq!(messages.lock().unwrap().len(), 1);
}

#[test]
fn removing_parent_directory_notifies_its_sources() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let targets = [target("/config/cache.yaml", messages.clone())];
    dispatch(
        &targets,
        &Event::new(EventKind::Remove(notify::event::RemoveKind::Folder))
            .add_path(PathBuf::from("/config")),
    );
    assert_eq!(messages.lock().unwrap().len(), 1);
}

#[test]
fn rescan_notifies_all_owners_without_specific_paths() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let targets = [target("/config/cache.yaml", messages.clone())];
    dispatch(
        &targets,
        &Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan),
    );
    assert_eq!(messages.lock().unwrap().len(), 1);
}
