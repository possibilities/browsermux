use crate::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: u32 = 1;
const DATABASE_NAME: &str = "session.sqlite3";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    schema_version: u32,
    workspace: Workspace,
    viewport: Size,
    containers: Vec<Container>,
    panes: Vec<SavedPane>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SavedPane {
    Persistent {
        id: Id,
        container_id: Id,
        tabs: Vec<SavedTab>,
        active_tab_id: Id,
    },
    TemporaryPlaceholder {
        id: Id,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedTab {
    id: Id,
    url: String,
}

pub(crate) fn export(state: &State) -> Result<String, CoreError> {
    state.validate()?;
    let panes = state
        .panes
        .values()
        .map(|p| match state.containers[&p.container_id].persistence {
            Persistence::Temporary => SavedPane::TemporaryPlaceholder { id: p.id },
            Persistence::Persistent => SavedPane::Persistent {
                id: p.id,
                container_id: p.container_id,
                active_tab_id: p.active_tab_id,
                tabs: p
                    .tabs
                    .iter()
                    .map(|t| SavedTab {
                        id: t.id,
                        url: safe_reopen_url(&t.url, t.reopen_allowed)
                            .unwrap_or_else(|| "about:blank".into()),
                    })
                    .collect(),
            },
        })
        .collect();
    let session = Session {
        schema_version: SCHEMA_VERSION,
        workspace: state.workspace.clone(),
        viewport: state.viewport,
        containers: state
            .containers
            .values()
            .filter(|c| c.persistence == Persistence::Persistent)
            .cloned()
            .collect(),
        panes,
    };
    let json = serde_json::to_string(&session).map_err(|_| CoreError::InvalidJson)?;
    if json.len() > MAX_SESSION_BYTES {
        return Err(CoreError::ResourceLimit);
    }
    Ok(json)
}
pub(crate) fn import(json: &str) -> Result<State, CoreError> {
    if json.len() > MAX_SESSION_BYTES {
        return Err(CoreError::ResourceLimit);
    }
    // Inspect only the version first: newer schemas may add fields that this
    // version does not understand, and must never be treated as corruption.
    #[derive(Deserialize)]
    struct VersionHeader {
        schema_version: u32,
    }
    let header: VersionHeader = serde_json::from_str(json).map_err(|_| CoreError::InvalidJson)?;
    if header.schema_version != SCHEMA_VERSION {
        return Err(CoreError::UnsupportedSchema);
    }
    let session: Session = serde_json::from_str(json).map_err(|_| CoreError::InvalidJson)?;
    if session.schema_version != SCHEMA_VERSION {
        return Err(CoreError::UnsupportedSchema);
    }
    if session.panes.is_empty()
        || session.panes.len() > MAX_PANES
        || session.containers.len() > MAX_TABS
    {
        return Err(CoreError::ResourceLimit);
    }
    let mut containers = BTreeMap::new();
    for c in session.containers {
        if c.persistence != Persistence::Persistent {
            return Err(CoreError::InvalidState(
                "temporary container data is forbidden in session files".into(),
            ));
        }
        if containers.insert(c.id, c).is_some() {
            return Err(CoreError::InvalidState("duplicate container ID".into()));
        }
    }
    let mut panes = BTreeMap::new();
    let mut tab_count = 0;
    for saved in session.panes {
        let p = match saved {
            SavedPane::TemporaryPlaceholder { id } => {
                let c = Container::new(
                    "Temporary (restored blank)".into(),
                    Persistence::Temporary,
                    None,
                );
                let mut p = Pane::blank(c.id);
                p.id = id;
                containers.insert(c.id, c);
                p
            }
            SavedPane::Persistent {
                id,
                container_id,
                tabs,
                active_tab_id,
            } => {
                if !containers.contains_key(&container_id) {
                    return Err(CoreError::InvalidState(
                        "saved pane references a missing container; explicit repair is required"
                            .into(),
                    ));
                }
                tab_count += tabs.len();
                if tab_count > MAX_TABS {
                    return Err(CoreError::ResourceLimit);
                }
                let mut restored = Vec::new();
                for t in tabs {
                    let mut tab = Tab::with_id(t.id);
                    if t.url != "about:blank" {
                        tab.url = safe_reopen_url(&t.url, true).ok_or(CoreError::UnsafeUrl)?;
                        tab.reopen_allowed = true;
                        tab.lifecycle = TabLifecycle::Ready;
                    }
                    restored.push(tab);
                }
                Pane {
                    id,
                    container_id,
                    generation: 0,
                    tabs: restored,
                    active_tab_id,
                }
            }
        };
        if panes.insert(p.id, p).is_some() {
            return Err(CoreError::InvalidState("duplicate pane ID".into()));
        }
    }
    let state = State {
        revision: 0,
        workspace: session.workspace,
        containers,
        panes,
        viewport: session.viewport,
        switches: BTreeMap::new(),
        moves: BTreeMap::new(),
    };
    state.validate()?;
    Ok(state)
}

/// Owned single-writer persistence guard. Keep alive as long as CEF uses this root.
/// File locks are advisory; every supported app launch must acquire this guard.
pub struct SessionStore {
    connection: Connection,
    _root_lock: File,
    root: PathBuf,
    generation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryNotice {
    pub message: String,
    pub preserved_files: Vec<PathBuf>,
}
impl SessionStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, CoreError> {
        let (root, lock) = lock_root(root.as_ref())?;
        let connection = open_database(&root)?;
        Ok(Self {
            connection,
            _root_lock: lock,
            root,
            generation: 0,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn load(&mut self) -> Result<Option<String>, CoreError> {
        let row: Option<(i64, Option<String>)> = self
            .connection
            .query_row(
                "SELECT generation,CASE WHEN length(CAST(payload AS BLOB)) <= ?1 THEN payload ELSE NULL END FROM session WHERE id=1",
                [MAX_SESSION_BYTES as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        match row {
            Some((generation, payload)) => {
                let payload = payload.ok_or(CoreError::ResourceLimit)?;
                if generation < 0 {
                    return Err(CoreError::InvalidState("negative stored generation".into()));
                }
                if payload.len() > MAX_SESSION_BYTES {
                    return Err(CoreError::ResourceLimit);
                }
                self.generation = generation as u64;
                Ok(Some(payload))
            }
            None => {
                self.generation = 0;
                Ok(None)
            }
        }
    }
    pub fn save(&mut self, json: &str) -> Result<(), CoreError> {
        // The storage layer independently rejects Snapshot JSON and unsanitized imports.
        import(json)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let current: Option<i64> = transaction
            .query_row("SELECT generation FROM session WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_error)?;
        if current.unwrap_or(0) < 0 || current.unwrap_or(0) as u64 != self.generation {
            return Err(CoreError::PersistenceConflict);
        }
        let next = self
            .generation
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or(CoreError::CounterOverflow)?;
        transaction.execute("INSERT OR REPLACE INTO previous_session(id,generation,payload) SELECT id,generation,payload FROM session WHERE id=1",[]).map_err(db_error)?;
        transaction.execute("INSERT INTO session(id,generation,payload) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET generation=excluded.generation,payload=excluded.payload",params![next as i64,json]).map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        self.generation = next;
        Ok(())
    }
    pub fn open_recovering(
        root: impl AsRef<Path>,
    ) -> Result<(Self, Option<String>, Option<RecoveryNotice>), CoreError> {
        let (root, lock) = lock_root(root.as_ref())?;
        let opened = open_database(&root);
        let (failure, lock) = match opened {
            Ok(connection) => {
                let mut store = Self {
                    connection,
                    _root_lock: lock,
                    root: root.clone(),
                    generation: 0,
                };
                match store.load().and_then(|json| {
                    if let Some(ref j) = json {
                        import(j)?;
                    }
                    Ok(json)
                }) {
                    Ok(json) => return Ok((store, json, None)),
                    Err(CoreError::UnsupportedSchema) => return Err(CoreError::UnsupportedSchema),
                    Err(error) if recoverable(&error) => {
                        let Self {
                            connection,
                            _root_lock,
                            ..
                        } = store;
                        drop(connection);
                        (error.code().to_owned(), _root_lock)
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(CoreError::UnsupportedSchema) => return Err(CoreError::UnsupportedSchema),
            Err(error) if recoverable(&error) => (error.code().to_owned(), lock),
            Err(error) => return Err(error),
        };
        // All connections have closed before renaming; each original byte sequence
        // remains available for user-controlled diagnosis, never erased automatically.
        let suffix = Id::new().to_string();
        let mut preserved = Vec::new();
        for name in [DATABASE_NAME, "session.sqlite3-wal", "session.sqlite3-shm"] {
            let from = root.join(name);
            if from.exists() {
                let to = root.join(format!("{name}.corrupt-{suffix}"));
                fs::rename(&from, &to).map_err(io_error)?;
                preserved.push(to);
            }
        }
        let connection = open_database(&root)?;
        let store = Self {
            connection,
            _root_lock: lock,
            root,
            generation: 0,
        };
        let notice = RecoveryNotice {
            message: format!(
                "The saved session could not be restored ({}). Its files were preserved; a blank temporary workspace was opened.",
                failure
            ),
            preserved_files: preserved,
        };
        Ok((store, None, Some(notice)))
    }
}
fn lock_root(root: &Path) -> Result<(PathBuf, File), CoreError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(root).map_err(io_error)?;
    let root = fs::canonicalize(root).map_err(io_error)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(root.join(".writer.lock")).map_err(io_error)?;
    lock.try_lock().map_err(|_| CoreError::DataRootLocked)?;
    Ok((root, lock))
}
fn open_database(root: &Path) -> Result<Connection, CoreError> {
    let path = root.join(DATABASE_NAME);
    let connection = Connection::open(&path).map_err(db_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(io_error)?;
    }
    let integrity: String = connection
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(db_error)?;
    if integrity != "ok" {
        return Err(CoreError::CorruptDatabase);
    }
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_error)?;
    if version > SCHEMA_VERSION {
        return Err(CoreError::UnsupportedSchema);
    }
    connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;
        CREATE TABLE IF NOT EXISTS session(id INTEGER PRIMARY KEY CHECK(id=1),generation INTEGER NOT NULL CHECK(generation>=0),payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS previous_session(id INTEGER PRIMARY KEY CHECK(id=1),generation INTEGER NOT NULL CHECK(generation>=0),payload TEXT NOT NULL);
        PRAGMA user_version=1;").map_err(db_error)?;
    Ok(connection)
}
// Database errors are structural diagnostics, not full URLs or browsing data.
fn recoverable(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::InvalidJson
            | CoreError::InvalidState(_)
            | CoreError::UnsafeUrl
            | CoreError::ResourceLimit
            | CoreError::CorruptDatabase
    )
}
fn db_error(error: rusqlite::Error) -> CoreError {
    match &error {
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            ) =>
        {
            CoreError::CorruptDatabase
        }
        rusqlite::Error::InvalidColumnType(..) | rusqlite::Error::IntegralValueOutOfRange(..) => {
            CoreError::InvalidState("invalid database metadata type".into())
        }
        _ => CoreError::Persistence(error.to_string()),
    }
}
fn io_error(error: std::io::Error) -> CoreError {
    CoreError::Persistence(error.to_string())
}

/// Sanitized immutable snapshot prepared under the model lock, then saved after
/// releasing that lock. Construction is restricted to BrowserCore.
#[derive(Clone, Debug)]
pub struct PreparedSession {
    pub(crate) revision: u64,
    pub(crate) payload: String,
}
impl PreparedSession {
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Separate writer ownership lets a bridge keep slow disk I/O outside its model
/// mutex. Serialize calls to this object, e.g. with a dedicated worker or mutex.
/// It retains the exclusive application-root lock independently of BrowserCore.
pub struct SessionWriter {
    pub(crate) store: SessionStore,
    pub(crate) last_revision: Option<u64>,
}
impl SessionWriter {
    /// Returns false if this revision was already saved or a newer one won the
    /// race. An older queued save can never overwrite a final shutdown snapshot.
    pub fn save(&mut self, prepared: &PreparedSession) -> Result<bool, CoreError> {
        if self
            .last_revision
            .is_some_and(|revision| revision >= prepared.revision)
        {
            return Ok(false);
        }
        self.store.save(&prepared.payload)?;
        self.last_revision = Some(prepared.revision);
        Ok(true)
    }
    pub fn last_saved_revision(&self) -> Option<u64> {
        self.last_revision
    }
}
