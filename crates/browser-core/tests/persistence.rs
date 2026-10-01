use browser_core::*;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::PathBuf;

struct TempRoot(PathBuf);
impl TempRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("browser-core-tests-{}", Id::new()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn persistent(core: &mut BrowserCore) -> (Id, Id) {
    let result = core
        .dispatch(Command::CreateContainer {
            name: "Work".into(),
            persistence: Persistence::Persistent,
            color: None,
        })
        .unwrap();
    let id = result
        .effects
        .iter()
        .find_map(|e| match e {
            Effect::ContainerCreated { container_id } => Some(*container_id),
            _ => None,
        })
        .unwrap();
    let result = core
        .dispatch(Command::BeginContainerSwitch {
            pane_id: None,
            destination_container: id,
        })
        .unwrap();
    let switch = result.snapshot.pending_switches[0].id;
    core.dispatch(Command::CommitContainerSwitch {
        switch_id: switch,
        confirmed: true,
        before_unload_resolved: true,
        replacements_ready: true,
    })
    .unwrap();
    (id, core.snapshot().panes[0].active_tab_id)
}
fn commit(core: &mut BrowserCore, tab: Id, url: &str) {
    core.dispatch(Command::Navigate {
        tab_id: Some(tab),
        url: url.into(),
    })
    .unwrap();
    let generation = core
        .snapshot()
        .panes
        .iter()
        .flat_map(|p| &p.tabs)
        .find(|t| t.id == tab)
        .unwrap()
        .navigation_generation;
    core.dispatch(Command::NavigationCommitted {
        tab_id: tab,
        expected_generation: generation,
        url: url.into(),
        title: "PRIVATE PAGE TITLE".into(),
        reopen_allowed: true,
    })
    .unwrap();
}

#[test]
fn file_root_lock_blocks_second_app_until_first_releases() {
    let root = TempRoot::new();
    let first = BrowserCore::open(&root.0).unwrap();
    assert!(matches!(
        BrowserCore::open(&root.0),
        Err(CoreError::DataRootLocked)
    ));
    assert!(matches!(
        BrowserCore::open_or_recover(&root.0),
        Err(CoreError::DataRootLocked)
    ));
    drop(first);
    BrowserCore::open(&root.0).unwrap();
}
#[test]
fn atomic_sqlite_round_trip_and_lazy_safe_metadata() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    let (container, tab) = persistent(&mut core);
    commit(&mut core, tab, "https://example.com/docs");
    let pane = core.snapshot().workspace.focused_pane;
    assert!(core.is_dirty());
    core.flush().unwrap();
    assert!(!core.is_dirty());
    drop(core);
    let restored = BrowserCore::open(&root.0).unwrap();
    let s = restored.snapshot();
    assert_eq!(s.workspace.focused_pane, pane);
    assert_eq!(s.panes[0].container_id, container);
    assert_eq!(s.panes[0].tabs[0].id, tab);
    assert_eq!(s.panes[0].tabs[0].url, "https://example.com/docs");
    assert!(s.panes[0].tabs[0].title.is_empty());
}
#[test]
fn unflushed_layout_changes_do_not_replace_last_committed_session() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    persistent(&mut core);
    core.flush().unwrap();
    let before = core.export_session().unwrap();
    core.dispatch(Command::Split {
        axis: Axis::LeftRight,
        pane_id: None,
    })
    .unwrap();
    assert_eq!(core.snapshot().panes.len(), 2);
    drop(core);
    let restored = BrowserCore::open(&root.0).unwrap();
    assert_eq!(restored.export_session().unwrap(), before);
}
#[test]
fn no_temporary_browsing_bytes_ever_enter_sqlite_or_wal() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    let snapshot = core.snapshot();
    let tab = snapshot.panes[0].active_tab_id;
    let container = snapshot.panes[0].container_id;
    core.dispatch(Command::RenameContainer {
        container_id: container,
        name: "TEMPORARY_SECRET_CONTAINER_829357".into(),
        color: None,
    })
    .unwrap();
    commit(
        &mut core,
        tab,
        "https://private.example/TEMPORARY_SECRET_URL_629153",
    );
    for _ in 0..4 {
        core.flush().unwrap();
    }
    for file in std::fs::read_dir(&root.0).unwrap() {
        let path = file.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            for forbidden in [
                "TEMPORARY_SECRET_CONTAINER_829357",
                "TEMPORARY_SECRET_URL_629153",
                "PRIVATE PAGE TITLE",
                &tab.to_string(),
                &container.to_string(),
            ] {
                assert!(
                    !text.contains(forbidden),
                    "{} contains {forbidden}",
                    path.display()
                );
            }
        }
    }
}
#[test]
fn sqlite_rejects_out_of_band_generation_conflict() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    core.flush().unwrap();
    let connection = Connection::open(root.0.join("session.sqlite3")).unwrap();
    connection
        .execute("UPDATE session SET generation=99", [])
        .unwrap();
    assert!(matches!(core.flush(), Err(CoreError::PersistenceConflict)));
}
#[test]
fn recovery_preserves_corrupt_database_and_retains_root_lock() {
    let root = TempRoot::new();
    let database = root.0.join("session.sqlite3");
    let original = b"a damaged database file that must not disappear";
    std::fs::write(&database, original).unwrap();
    assert!(BrowserCore::open(&root.0).is_err());
    assert_eq!(std::fs::read(&database).unwrap(), original);
    let (mut core, notice) = BrowserCore::open_or_recover(&root.0).unwrap();
    let notice = notice.unwrap();
    assert!(!notice.preserved_files.is_empty());
    assert_eq!(std::fs::read(&notice.preserved_files[0]).unwrap(), original);
    assert_eq!(core.snapshot().panes[0].tabs[0].url, "about:blank");
    assert!(matches!(
        BrowserCore::open(&root.0),
        Err(CoreError::DataRootLocked)
    ));
    core.flush().unwrap();
    drop(core);
    BrowserCore::open(&root.0).unwrap();
}
#[test]
fn invalid_payload_is_preserved_and_missing_container_not_reassigned() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    persistent(&mut core);
    core.flush().unwrap();
    let mut bad: Value = serde_json::from_str(&core.export_session().unwrap()).unwrap();
    bad["containers"] = json!([]);
    drop(core);
    let connection = Connection::open(root.0.join("session.sqlite3")).unwrap();
    connection
        .execute("UPDATE session SET payload=?1", [bad.to_string()])
        .unwrap();
    drop(connection);
    let (core, notice) = BrowserCore::open_or_recover(&root.0).unwrap();
    assert!(notice.is_some());
    assert!(
        core.snapshot()
            .containers
            .iter()
            .all(|c| c.persistence == Persistence::Temporary)
    );
    assert_eq!(core.snapshot().panes[0].tabs[0].url, "about:blank");
}
#[test]
fn future_database_and_session_versions_are_never_downgraded() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    core.flush().unwrap();
    drop(core);
    let connection = Connection::open(root.0.join("session.sqlite3")).unwrap();
    connection.execute_batch("PRAGMA user_version=999").unwrap();
    drop(connection);
    assert!(matches!(
        BrowserCore::open_or_recover(&root.0),
        Err(CoreError::UnsupportedSchema)
    ));
    let connection = Connection::open(root.0.join("session.sqlite3")).unwrap();
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 999);
    assert!(
        !std::fs::read_dir(&root.0).unwrap().any(|f| f
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("corrupt"))
    );
}
#[test]
fn session_store_rejects_unsanitized_untrusted_payload() {
    let root = TempRoot::new();
    let mut core = BrowserCore::new();
    persistent(&mut core);
    let mut value: Value = serde_json::from_str(&core.export_session().unwrap()).unwrap();
    value["panes"][0]["tabs"][0]["url"] = json!("https://example.com/?secret=do-not-store");
    let mut store = SessionStore::open(&root.0).unwrap();
    assert!(store.save(&value.to_string()).is_err());
    assert!(store.load().unwrap().is_none());
}
#[test]
fn previous_snapshot_is_always_sanitized_and_atomic() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    let (_, tab) = persistent(&mut core);
    commit(&mut core, tab, "https://example.com/first");
    core.flush().unwrap();
    commit(&mut core, tab, "https://example.com/second");
    core.flush().unwrap();
    let connection = Connection::open(root.0.join("session.sqlite3")).unwrap();
    let (previous, generation): (String, i64) = connection
        .query_row(
            "SELECT payload,generation FROM previous_session WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(previous.contains("/first"));
    assert!(!previous.contains("/second"));
    assert_eq!(generation, 1);
    BrowserCore::from_session(&previous).unwrap();
}
#[test]
fn large_and_unknown_session_fields_fail_closed() {
    let core = BrowserCore::new();
    let mut value: Value = serde_json::from_str(&core.export_session().unwrap()).unwrap();
    value["cookies"] = json!({"session":"secret"});
    assert!(BrowserCore::from_session(&value.to_string()).is_err());
    assert!(matches!(
        BrowserCore::from_session(&" ".repeat(MAX_SESSION_BYTES + 1)),
        Err(CoreError::ResourceLimit)
    ));
}

#[test]
fn future_payload_with_new_fields_is_preserved_without_recovery_rotation() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    core.flush().unwrap();
    let mut value: Value = serde_json::from_str(&core.export_session().unwrap()).unwrap();
    value["schema_version"] = json!(2);
    value["future_field"] = json!({"new_structure":[1,2,3]});
    drop(core);
    let connection = Connection::open(root.0.join("session.sqlite3")).unwrap();
    connection
        .execute("UPDATE session SET payload=?1", [value.to_string()])
        .unwrap();
    drop(connection);
    assert!(matches!(
        BrowserCore::open_or_recover(&root.0),
        Err(CoreError::UnsupportedSchema)
    ));
    assert!(
        !std::fs::read_dir(&root.0).unwrap().any(|f| f
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("corrupt"))
    );
    let connection = Connection::open(root.0.join("session.sqlite3")).unwrap();
    let unchanged: String = connection
        .query_row("SELECT payload FROM session", [], |r| r.get(0))
        .unwrap();
    assert_eq!(unchanged, value.to_string());
}
#[test]
fn sqlite_write_contention_is_not_treated_as_database_corruption() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    core.flush().unwrap();
    drop(core);
    let connection = Connection::open(root.0.join("session.sqlite3")).unwrap();
    connection
        .execute_batch("BEGIN EXCLUSIVE; UPDATE session SET generation=generation;")
        .unwrap();
    assert!(matches!(
        BrowserCore::open_or_recover(&root.0),
        Err(CoreError::Persistence(_))
    ));
    assert!(
        !std::fs::read_dir(&root.0).unwrap().any(|f| f
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("corrupt"))
    );
    connection.execute_batch("ROLLBACK").unwrap();
    drop(connection);
    BrowserCore::open(&root.0).unwrap();
}
#[test]
fn oversized_database_payload_is_bounded_before_materialization() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    core.flush().unwrap();
    drop(core);
    let connection = Connection::open(root.0.join("session.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE session SET payload=zeroblob(?1)",
            [MAX_SESSION_BYTES as i64 + 1],
        )
        .unwrap();
    drop(connection);
    assert!(matches!(
        BrowserCore::open(&root.0),
        Err(CoreError::ResourceLimit)
    ));
}

#[test]
fn detached_writer_rejects_out_of_order_snapshots_and_preserves_dirty_edits() {
    let root = TempRoot::new();
    let mut core = BrowserCore::open(&root.0).unwrap();
    persistent(&mut core);
    let mut writer = core.detach_persistence().unwrap();
    let old = core.prepare_session().unwrap();
    core.dispatch(Command::NewTab { pane_id: None }).unwrap();
    let latest = core.prepare_session().unwrap();
    assert!(writer.save(&latest).unwrap());
    assert!(!writer.save(&old).unwrap());
    assert!(!writer.save(&latest).unwrap());
    assert!(core.mark_session_saved(latest.revision()));
    assert!(!core.is_dirty());
    core.dispatch(Command::NewTab { pane_id: None }).unwrap();
    assert!(!core.mark_session_saved(latest.revision()));
    assert!(core.is_dirty());
    drop(core);
    assert!(matches!(
        BrowserCore::open(&root.0),
        Err(CoreError::DataRootLocked)
    ));
    drop(writer);
    let restored = BrowserCore::open(&root.0).unwrap();
    assert_eq!(restored.snapshot().panes[0].tabs.len(), 2);
}
