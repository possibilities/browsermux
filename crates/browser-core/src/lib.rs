//! Engine-independent native browser state. All commands are transactional and return
//! explicit native-engine effects; this crate itself cannot browse or transfer storage.
mod command;
mod model;
mod persistence;
pub use command::*;
pub use model::*;
pub use persistence::{PreparedSession, RecoveryNotice, SessionStore, SessionWriter};
use std::path::Path;
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("{0}")]
    InvalidState(String),
    #[error("The requested object no longer exists")]
    NotFound,
    #[error("The operation no longer matches the current pane or navigation")]
    StaleGeneration,
    #[error("Confirmation and native beforeunload resolution are required")]
    ConfirmationRequired,
    #[error("The replacement browser instances are not ready")]
    ReplacementsNotReady,
    #[error("The pane is too small to split (minimum content size is 240 × 160)")]
    InsufficientSpace,
    #[error("That pane already has a pending replacement operation")]
    Busy,
    #[error("A different container requires an explicitly confirmed fresh browser instance")]
    CrossContainerMove,
    #[error("Only well-formed HTTP, HTTPS and about:blank navigation is permitted")]
    UnsafeUrl,
    #[error("No divider is available in that direction")]
    NoDivider,
    #[error("The supported resource safety limit has been reached")]
    ResourceLimit,
    #[error("The application data root is already in use by another process")]
    DataRootLocked,
    #[error("The saved session changed since it was opened")]
    PersistenceConflict,
    #[error("Unsupported session schema version")]
    UnsupportedSchema,
    #[error("Invalid JSON request or session data")]
    InvalidJson,
    #[error("The session database is damaged")]
    CorruptDatabase,
    #[error("Persistence failed: {0}")]
    Persistence(String),
    #[error("A counter exhausted its supported range")]
    CounterOverflow,
}
impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidState(_) => "invalid_state",
            Self::NotFound => "not_found",
            Self::StaleGeneration => "stale_generation",
            Self::ConfirmationRequired => "confirmation_required",
            Self::ReplacementsNotReady => "replacements_not_ready",
            Self::InsufficientSpace => "insufficient_space",
            Self::Busy => "busy",
            Self::CrossContainerMove => "cross_container_move",
            Self::UnsafeUrl => "unsafe_url",
            Self::NoDivider => "no_divider",
            Self::ResourceLimit => "resource_limit",
            Self::DataRootLocked => "data_root_locked",
            Self::PersistenceConflict => "persistence_conflict",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::InvalidJson => "invalid_json",
            Self::Persistence(_) => "persistence_error",
            Self::CorruptDatabase => "corrupt_database",
            Self::CounterOverflow => "counter_overflow",
        }
    }
}

pub struct BrowserCore {
    state: State,
    store: Option<SessionStore>,
    dirty: bool,
}
impl Default for BrowserCore {
    fn default() -> Self {
        Self::new()
    }
}
impl BrowserCore {
    pub fn new() -> Self {
        Self {
            state: State::fresh(),
            store: None,
            dirty: false,
        }
    }
    pub fn open(data_root: impl AsRef<Path>) -> Result<Self, CoreError> {
        let mut store = SessionStore::open(data_root)?;
        let state = match store.load()? {
            Some(json) => persistence::import(&json)?,
            None => State::fresh(),
        };
        Ok(Self {
            state,
            store: Some(store),
            dirty: false,
        })
    }
    /// Preserve corrupt session bytes and retain the root lock while using a safe blank model.
    /// Unsupported schemas are NOT overwritten or migrated speculatively.
    pub fn open_or_recover(
        data_root: impl AsRef<Path>,
    ) -> Result<(Self, Option<RecoveryNotice>), CoreError> {
        let (store, json, notice) = SessionStore::open_recovering(data_root)?;
        let state = match json {
            Some(json) => persistence::import(&json)?,
            None => State::fresh(),
        };
        Ok((
            Self {
                state,
                store: Some(store),
                dirty: false,
            },
            notice,
        ))
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.snapshot()
    }
    pub fn dispatch_json(&mut self, json: &str) -> Result<CommandResult, CoreError> {
        if json.len() > MAX_SESSION_BYTES {
            return Err(CoreError::ResourceLimit);
        }
        let command = serde_json::from_str(json).map_err(|_| CoreError::InvalidJson)?;
        self.dispatch(command)
    }
    pub fn dispatch(&mut self, command: Command) -> Result<CommandResult, CoreError> {
        let mut next = self.state.clone();
        let mut effects = Vec::new();
        next.apply(command, &mut effects)?;
        next.validate()?;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(CoreError::CounterOverflow)?;
        self.state = next;
        self.dirty = true;
        Ok(CommandResult {
            snapshot: self.snapshot(),
            effects,
        })
    }
    pub fn export_session(&self) -> Result<String, CoreError> {
        persistence::export(&self.state)
    }
    pub fn from_session(json: &str) -> Result<Self, CoreError> {
        Ok(Self {
            state: persistence::import(json)?,
            store: None,
            dirty: false,
        })
    }
    pub fn flush(&mut self) -> Result<(), CoreError> {
        let json = self.export_session()?;
        let store = self
            .store
            .as_mut()
            .ok_or_else(|| CoreError::Persistence("this model has no session store".into()))?;
        store.save(&json)?;
        self.dirty = false;
        Ok(())
    }
    /// Move the root lock and SQLite writer into a separate persistence owner.
    /// The returned writer must remain alive until the native engine shuts down.
    pub fn detach_persistence(&mut self) -> Option<SessionWriter> {
        self.store.take().map(|store| SessionWriter {
            store,
            last_revision: None,
        })
    }
    pub fn prepare_session(&self) -> Result<PreparedSession, CoreError> {
        Ok(PreparedSession {
            revision: self.state.revision,
            payload: self.export_session()?,
        })
    }
    /// A background save may complete after another command. Clear dirty state
    /// only when it saved the current revision, never a newer unsaved edit.
    pub fn mark_session_saved(&mut self, revision: u64) -> bool {
        if revision == self.state.revision {
            self.dirty = false;
            true
        } else {
            false
        }
    }
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn validate(&self) -> Result<(), CoreError> {
        self.state.validate()
    }
}

pub(crate) fn validate_label(name: &str) -> Result<(), CoreError> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err(CoreError::InvalidState(
            "container name must be 1–256 bytes without controls".into(),
        ));
    }
    Ok(())
}
pub(crate) fn validate_color(color: Option<&str>) -> Result<(), CoreError> {
    if color.is_some_and(|c| {
        c.len() != 7 || !c.starts_with('#') || !c.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    }) {
        return Err(CoreError::InvalidState(
            "container color must be #RRGGBB".into(),
        ));
    }
    Ok(())
}
/// Validate explicit top-level navigation. Search expansion is a separate UI decision.
pub fn validate_navigation(input: &str) -> Result<String, CoreError> {
    if input == "about:blank" {
        return Ok(input.into());
    }
    if input.len() > MAX_URL_LENGTH
        || input.chars().any(char::is_control)
        || input.contains('\\')
        || input.trim() != input
    {
        return Err(CoreError::UnsafeUrl);
    }
    let url = Url::parse(input).map_err(|_| CoreError::UnsafeUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(CoreError::UnsafeUrl);
    }
    Ok(url.into())
}
/// Conservative automatic-reopen policy. Query/fragment data and authentication or
/// token-like path components remain blank even after broad confirmation.
pub fn safe_reopen_url(input: &str, reopen_allowed: bool) -> Option<String> {
    if !reopen_allowed {
        return None;
    }
    let normalized = validate_navigation(input).ok()?;
    let url = Url::parse(&normalized).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let path = url.path().to_ascii_lowercase();
    let forbidden = [
        "token",
        "oauth",
        "authorize",
        "callback",
        "magic",
        "reset",
        "verify",
        "verification",
        "activation",
        "activate",
        "logout",
        "signout",
        "unsubscribe",
        "confirm",
    ];
    if forbidden.iter().any(|word| path.contains(word))
        || path.contains('%')
        || url
            .path_segments()
            .is_some_and(|parts| parts.into_iter().any(|part| part.len() > 64))
    {
        return None;
    }
    Some(normalized)
}
fn replacement(t: &Tab) -> Replacement {
    let url = safe_reopen_url(&t.url, t.reopen_allowed);
    let parsed = url.as_ref().and_then(|u| Url::parse(u).ok());
    Replacement {
        source_tab_id: t.id,
        source_generation: t.navigation_generation,
        source_url: t.url.clone(),
        replacement_tab_id: Id::new(),
        preview_origin: parsed.as_ref().map(|u| u.origin().ascii_serialization()),
        preview_path: parsed.as_ref().map(|u| u.path().to_owned()),
        url,
    }
}
fn bump(value: &mut u64) -> Result<(), CoreError> {
    *value = value.checked_add(1).ok_or(CoreError::CounterOverflow)?;
    Ok(())
}
fn create_effect(tab: &Tab, container_id: Id) -> Effect {
    Effect::CreateTab {
        tab_id: tab.id,
        container_id,
        url: "about:blank".into(),
    }
}
impl State {
    fn pane_id(&self, id: Option<Id>) -> Result<Id, CoreError> {
        let id = id.unwrap_or(self.workspace.focused_pane);
        self.panes
            .contains_key(&id)
            .then_some(id)
            .ok_or(CoreError::NotFound)
    }
    fn tab_id(&self, id: Option<Id>) -> Result<Id, CoreError> {
        match id {
            Some(id) => self.tab_location(id).map(|_| id).ok_or(CoreError::NotFound),
            None => Ok(self.panes[&self.workspace.focused_pane].active_tab_id),
        }
    }
    fn focus(&mut self, id: Id) -> Result<(), CoreError> {
        self.pane_id(Some(id))?;
        if self.workspace.focused_pane != id {
            self.workspace.last_focused_pane = Some(self.workspace.focused_pane);
            self.workspace.focused_pane = id;
        }
        if self.workspace.zoomed_pane.is_some() {
            self.workspace.zoomed_pane = Some(id);
        }
        Ok(())
    }
    fn require_idle(&self, id: Id) -> Result<(), CoreError> {
        if self.switches.values().any(|p| p.pane_id == id)
            || self
                .moves
                .values()
                .any(|p| p.source_pane == id || p.destination_pane == id)
        {
            return Err(CoreError::Busy);
        }
        Ok(())
    }
    fn tab_mut(&mut self, id: Id) -> Result<&mut Tab, CoreError> {
        let (p, i) = self.tab_location(id).ok_or(CoreError::NotFound)?;
        Ok(&mut self.panes.get_mut(&p).expect("validated pane").tabs[i])
    }
    fn navigate(
        &mut self,
        id: Id,
        url: String,
        effects: &mut Vec<Effect>,
    ) -> Result<(), CoreError> {
        let url = validate_navigation(&url)?;
        let (p, i) = self.tab_location(id).ok_or(CoreError::NotFound)?;
        let pane = self.panes.get_mut(&p).expect("known pane");
        let tab = &mut pane.tabs[i];
        bump(&mut tab.navigation_generation)?;
        tab.url = url.clone();
        tab.title.clear();
        tab.lifecycle = if url == "about:blank" {
            TabLifecycle::Blank
        } else {
            TabLifecycle::Loading
        };
        tab.reopen_allowed = false;
        effects.push(Effect::Navigate {
            tab_id: id,
            container_id: pane.container_id,
            url,
            navigation_generation: tab.navigation_generation,
        });
        Ok(())
    }
    fn close_tab(
        &mut self,
        pane_id: Id,
        tab_id: Id,
        effects: &mut Vec<Effect>,
    ) -> Result<(), CoreError> {
        let pane = self.panes.get_mut(&pane_id).ok_or(CoreError::NotFound)?;
        let index = pane
            .tabs
            .iter()
            .position(|t| t.id == tab_id)
            .ok_or(CoreError::NotFound)?;
        pane.tabs.remove(index);
        effects.push(Effect::CloseTab {
            tab_id,
            container_id: pane.container_id,
        });
        if pane.tabs.is_empty() {
            let tab = Tab::blank();
            pane.active_tab_id = tab.id;
            effects.push(create_effect(&tab, pane.container_id));
            pane.tabs.push(tab);
        } else if pane.active_tab_id == tab_id {
            pane.active_tab_id = pane.tabs[index.min(pane.tabs.len() - 1)].id;
        }
        bump(&mut pane.generation)?;
        Ok(())
    }
    fn retire_unused_temporary(&mut self, effects: &mut Vec<Effect>) {
        let dead: Vec<Id> = self
            .containers
            .values()
            .filter(|c| {
                c.persistence == Persistence::Temporary
                    && !self.panes.values().any(|p| p.container_id == c.id)
                    && !self
                        .switches
                        .values()
                        .any(|s| s.destination_container == c.id)
                    && !self.moves.values().any(|s| s.destination_container == c.id)
            })
            .map(|c| c.id)
            .collect();
        for id in dead {
            self.containers.remove(&id);
            effects.push(Effect::RetireContainer { container_id: id });
        }
    }
    fn resize(&mut self, id: Id, ratio: f64) -> Result<(), CoreError> {
        if !ratio.is_finite() {
            return Err(CoreError::InvalidState(
                "divider ratio must be finite".into(),
            ));
        }
        let s = self.content_size();
        let (rect, axis, a, b) = model::split_geometry(
            &self.workspace.root,
            id,
            Rect {
                x: 0.0,
                y: 0.0,
                width: s.width,
                height: s.height,
            },
        )
        .ok_or(CoreError::NotFound)?;
        let (space, lo, hi) = match axis {
            Axis::LeftRight => (rect.width - DIVIDER_SIZE, a.width, b.width),
            Axis::TopBottom => (rect.height - DIVIDER_SIZE, a.height, b.height),
        };
        let low = lo / space;
        let high = (1.0 - hi / space).max(low);
        let clamped = ratio.clamp(low, high);
        if let Some(Node::Split { ratio, .. }) = self.workspace.root.split_mut(id) {
            *ratio = clamped;
        }
        Ok(())
    }
    fn focus_direction(&mut self, direction: Direction) -> Result<(), CoreError> {
        let layout = self.base_layout();
        let current = layout
            .iter()
            .find(|p| p.pane_id == self.workspace.focused_pane)
            .ok_or(CoreError::NotFound)?
            .rect;
        let mut best: Option<(Id, f64, f64)> = None;
        for item in layout {
            if item.pane_id == self.workspace.focused_pane {
                continue;
            }
            let r = item.rect;
            let qualifies = match direction {
                Direction::Left => r.x + r.width <= current.x + 0.001,
                Direction::Right => r.x >= current.x + current.width - 0.001,
                Direction::Up => r.y + r.height <= current.y + 0.001,
                Direction::Down => r.y >= current.y + current.height - 0.001,
            };
            if !qualifies {
                continue;
            }
            let overlap = match direction.axis() {
                Axis::LeftRight => {
                    (r.y + r.height).min(current.y + current.height) - r.y.max(current.y)
                }
                Axis::TopBottom => {
                    (r.x + r.width).min(current.x + current.width) - r.x.max(current.x)
                }
            }
            .max(0.0);
            let horizontal_gap = (r.x - current.x - current.width)
                .max(current.x - r.x - r.width)
                .max(0.0);
            let vertical_gap = (r.y - current.y - current.height)
                .max(current.y - r.y - r.height)
                .max(0.0);
            let distance = horizontal_gap * horizontal_gap + vertical_gap * vertical_gap;
            if best.as_ref().is_none_or(|(_, o, d)| {
                overlap > *o + 0.0001 || ((overlap - *o).abs() < 0.0001 && distance < *d)
            }) {
                best = Some((item.pane_id, overlap, distance));
            }
        }
        if let Some((id, _, _)) = best {
            self.focus(id)?;
        }
        Ok(())
    }
    fn apply(&mut self, command: Command, effects: &mut Vec<Effect>) -> Result<(), CoreError> {
        match command {
            Command::CreateContainer {
                name,
                persistence,
                color,
            } => {
                validate_label(&name)?;
                validate_color(color.as_deref())?;
                let c = Container::new(name, persistence, color);
                effects.push(Effect::ContainerCreated { container_id: c.id });
                self.containers.insert(c.id, c);
            }
            Command::RenameContainer {
                container_id,
                name,
                color,
            } => {
                validate_label(&name)?;
                validate_color(color.as_deref())?;
                let c = self
                    .containers
                    .get_mut(&container_id)
                    .ok_or(CoreError::NotFound)?;
                c.name = name;
                c.color = color;
            }
            Command::SetViewport { width, height } => {
                self.viewport = Size { width, height };
            }
            Command::Split { axis, pane_id } => {
                let id = self.pane_id(pane_id)?;
                self.require_idle(id)?;
                if self.panes.len() >= MAX_PANES {
                    return Err(CoreError::ResourceLimit);
                }
                let rect = self
                    .base_layout()
                    .into_iter()
                    .find(|l| l.pane_id == id)
                    .ok_or(CoreError::NotFound)?
                    .rect;
                let fits = match axis {
                    Axis::LeftRight => rect.width >= MIN_PANE_WIDTH * 2.0 + DIVIDER_SIZE,
                    Axis::TopBottom => rect.height >= MIN_PANE_HEIGHT * 2.0 + DIVIDER_SIZE,
                };
                if !fits {
                    return Err(CoreError::InsufficientSpace);
                }
                let pane = Pane::blank(self.panes[&id].container_id);
                let new = pane.id;
                effects.push(create_effect(&pane.tabs[0], pane.container_id));
                self.workspace.root.split_leaf(id, new, axis);
                self.panes.insert(new, pane);
                self.focus(new)?;
            }
            Command::ClosePane { pane_id, confirmed } => {
                if !confirmed {
                    return Err(CoreError::ConfirmationRequired);
                }
                let id = self.pane_id(pane_id)?;
                self.require_idle(id)?;
                let old = self.panes.remove(&id).ok_or(CoreError::NotFound)?;
                effects.push(Effect::RevokePane { pane_id: id });
                for tab in &old.tabs {
                    effects.push(Effect::CloseTab {
                        tab_id: tab.id,
                        container_id: old.container_id,
                    });
                }
                if self.panes.is_empty() {
                    let pane = Pane::blank(old.container_id);
                    effects.push(create_effect(&pane.tabs[0], pane.container_id));
                    self.workspace.root = Node::Leaf { pane_id: pane.id };
                    self.workspace.focused_pane = pane.id;
                    self.workspace.last_focused_pane = None;
                    self.workspace.zoomed_pane = None;
                    self.panes.insert(pane.id, pane);
                } else {
                    self.workspace.root = self
                        .workspace
                        .root
                        .clone()
                        .remove_leaf(id)
                        .ok_or(CoreError::NotFound)?;
                    if self.workspace.focused_pane == id {
                        self.workspace.focused_pane = self
                            .workspace
                            .last_focused_pane
                            .filter(|p| self.panes.contains_key(p))
                            .unwrap_or(self.workspace.root.leaf_ids()[0]);
                    }
                    if self.workspace.last_focused_pane == Some(id) {
                        self.workspace.last_focused_pane = None;
                    }
                    if self.workspace.zoomed_pane == Some(id) {
                        self.workspace.zoomed_pane = None;
                    }
                }
                self.retire_unused_temporary(effects);
            }
            Command::FocusPane { pane_id } => self.focus(pane_id)?,
            Command::FocusDirection { direction } => self.focus_direction(direction)?,
            Command::FocusNext { backwards } => {
                let ids = self.workspace.root.leaf_ids();
                let at = ids
                    .iter()
                    .position(|id| *id == self.workspace.focused_pane)
                    .ok_or(CoreError::NotFound)?;
                let next = if backwards {
                    (at + ids.len() - 1) % ids.len()
                } else {
                    (at + 1) % ids.len()
                };
                self.focus(ids[next])?;
            }
            Command::FocusPrevious => {
                if let Some(id) = self.workspace.last_focused_pane {
                    self.focus(id)?;
                }
            }
            Command::ToggleZoom { pane_id } => {
                let id = self.pane_id(pane_id)?;
                if self.workspace.zoomed_pane == Some(id) {
                    self.workspace.zoomed_pane = None;
                } else {
                    self.focus(id)?;
                    self.workspace.zoomed_pane = Some(id);
                }
            }
            Command::SwapPanes { first, second } => {
                self.pane_id(Some(first))?;
                self.pane_id(Some(second))?;
                self.workspace.root.swap(first, second);
            }
            Command::SwapAdjacent { backwards } => {
                let ids = self.workspace.root.leaf_ids();
                let at = ids
                    .iter()
                    .position(|id| *id == self.workspace.focused_pane)
                    .ok_or(CoreError::NotFound)?;
                let next = if backwards {
                    (at + ids.len() - 1) % ids.len()
                } else {
                    (at + 1) % ids.len()
                };
                self.workspace.root.swap(ids[at], ids[next]);
            }
            Command::ResizeSplit { split_id, ratio } => self.resize(split_id, ratio)?,
            Command::ResizeDirection { direction, pixels } => {
                if !pixels.is_finite() || !(0.0..=100_000.0).contains(&pixels) {
                    return Err(CoreError::InvalidState("invalid resize amount".into()));
                }
                let mut ancestors = Vec::new();
                self.workspace
                    .root
                    .ancestors(self.workspace.focused_pane, &mut ancestors);
                let id = ancestors
                    .into_iter()
                    .rev()
                    .find(|(_, axis)| *axis == direction.axis())
                    .map(|(id, _)| id)
                    .ok_or(CoreError::NoDivider)?;
                let s = self.content_size();
                let (rect, axis, _, _) = model::split_geometry(
                    &self.workspace.root,
                    id,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: s.width,
                        height: s.height,
                    },
                )
                .ok_or(CoreError::NotFound)?;
                let span = match axis {
                    Axis::LeftRight => rect.width,
                    Axis::TopBottom => rect.height,
                } - DIVIDER_SIZE;
                let ratio = match self.workspace.root.split_mut(id) {
                    Some(Node::Split { ratio, .. }) => *ratio,
                    _ => return Err(CoreError::NoDivider),
                };
                self.resize(id, ratio + direction.sign() * pixels / span)?;
            }
            Command::NewTab { pane_id } => {
                let id = self.pane_id(pane_id)?;
                self.require_idle(id)?;
                let pane = self.panes.get_mut(&id).ok_or(CoreError::NotFound)?;
                let tab = Tab::blank();
                effects.push(create_effect(&tab, pane.container_id));
                pane.active_tab_id = tab.id;
                pane.tabs.push(tab);
                bump(&mut pane.generation)?;
            }
            Command::ActivateTab { pane_id, tab_id } => {
                let id = self.pane_id(pane_id)?;
                let pane = self.panes.get_mut(&id).ok_or(CoreError::NotFound)?;
                if !pane.tabs.iter().any(|t| t.id == tab_id) {
                    return Err(CoreError::NotFound);
                }
                pane.active_tab_id = tab_id;
                self.focus(id)?;
            }
            Command::CloseTab {
                pane_id,
                tab_id,
                confirmed,
            } => {
                if !confirmed {
                    return Err(CoreError::ConfirmationRequired);
                }
                let id = self.pane_id(pane_id)?;
                self.require_idle(id)?;
                let tab_id = tab_id.unwrap_or(self.panes[&id].active_tab_id);
                effects.push(Effect::RevokePane { pane_id: id });
                self.close_tab(id, tab_id, effects)?;
            }
            Command::Navigate { tab_id, url } => {
                let id = self.tab_id(tab_id)?;
                self.navigate(id, url, effects)?;
            }
            Command::NavigationStarted {
                tab_id,
                expected_generation,
            } => {
                let t = self.tab_mut(tab_id)?;
                if t.navigation_generation != expected_generation {
                    return Err(CoreError::StaleGeneration);
                }
                bump(&mut t.navigation_generation)?;
                t.lifecycle = TabLifecycle::Loading;
                t.reopen_allowed = false;
            }
            Command::NavigationCommitted {
                tab_id,
                expected_generation,
                url,
                title,
                reopen_allowed,
            } => {
                let url = validate_navigation(&url)?;
                if title.len() > 4096 || title.contains('\0') {
                    return Err(CoreError::InvalidState("invalid tab title".into()));
                }
                let t = self.tab_mut(tab_id)?;
                if t.navigation_generation != expected_generation {
                    return Err(CoreError::StaleGeneration);
                }
                t.url = url;
                t.title = title;
                t.lifecycle = if t.url == "about:blank" {
                    TabLifecycle::Blank
                } else {
                    TabLifecycle::Ready
                };
                t.reopen_allowed = reopen_allowed;
                t.has_before_unload = false;
            }
            Command::SetTabWarnings {
                tab_id,
                has_before_unload,
                active_downloads,
            } => {
                let t = self.tab_mut(tab_id)?;
                t.has_before_unload = has_before_unload;
                t.active_downloads = active_downloads;
            }
            Command::RendererFailed {
                tab_id,
                expected_generation,
            } => {
                let t = self.tab_mut(tab_id)?;
                if t.navigation_generation != expected_generation {
                    return Err(CoreError::StaleGeneration);
                }
                bump(&mut t.navigation_generation)?;
                t.lifecycle = TabLifecycle::Crashed;
                t.has_before_unload = false;
            }
            Command::MoveTab {
                tab_id,
                destination_pane,
                index,
            } => {
                let (source, i) = self.tab_location(tab_id).ok_or(CoreError::NotFound)?;
                self.pane_id(Some(destination_pane))?;
                self.require_idle(source)?;
                self.require_idle(destination_pane)?;
                if self.panes[&source].container_id != self.panes[&destination_pane].container_id {
                    return Err(CoreError::CrossContainerMove);
                }
                let tab = {
                    let p = self.panes.get_mut(&source).ok_or(CoreError::NotFound)?;
                    let tab = p.tabs.remove(i);
                    if source != destination_pane {
                        if p.tabs.is_empty() {
                            let blank = Tab::blank();
                            effects.push(create_effect(&blank, p.container_id));
                            p.active_tab_id = blank.id;
                            p.tabs.push(blank);
                        } else if p.active_tab_id == tab_id {
                            p.active_tab_id = p.tabs[i.min(p.tabs.len() - 1)].id;
                        }
                    }
                    bump(&mut p.generation)?;
                    tab
                };
                let p = self
                    .panes
                    .get_mut(&destination_pane)
                    .ok_or(CoreError::NotFound)?;
                let index = index.unwrap_or(p.tabs.len());
                if index > p.tabs.len() {
                    return Err(CoreError::InvalidState(
                        "tab insertion index out of range".into(),
                    ));
                }
                p.tabs.insert(index, tab);
                p.active_tab_id = tab_id;
                bump(&mut p.generation)?;
            }
            Command::BeginContainerSwitch {
                pane_id,
                destination_container,
            } => self.begin_switch(pane_id, destination_container, effects)?,
            Command::CommitContainerSwitch {
                switch_id,
                confirmed,
                before_unload_resolved,
                replacements_ready,
            } => self.commit_switch(
                switch_id,
                confirmed,
                before_unload_resolved,
                replacements_ready,
                effects,
            )?,
            Command::CancelContainerSwitch { switch_id } => {
                let plan = self
                    .switches
                    .remove(&switch_id)
                    .ok_or(CoreError::NotFound)?;
                for r in plan.replacements {
                    effects.push(Effect::CloseTab {
                        tab_id: r.replacement_tab_id,
                        container_id: plan.destination_container,
                    });
                }
                self.retire_unused_temporary(effects);
            }
            Command::BeginCrossContainerMove {
                tab_id,
                destination_pane,
            } => self.begin_move(tab_id, destination_pane, effects)?,
            Command::CommitCrossContainerMove {
                move_id,
                confirmed,
                replacement_ready,
            } => self.commit_move(move_id, confirmed, replacement_ready, effects)?,
            Command::CompleteCrossContainerMove {
                move_id,
                success,
                before_unload_resolved,
            } => self.complete_move(move_id, success, before_unload_resolved, effects)?,
            Command::CancelCrossContainerMove { move_id } => {
                let p = self.moves.remove(&move_id).ok_or(CoreError::NotFound)?;
                // Before commit only the prepared blank is discarded. After commit
                // cancellation preserves both tabs: requests cannot be rolled back.
                if !p.committed {
                    effects.push(Effect::CloseTab {
                        tab_id: p.replacement.replacement_tab_id,
                        container_id: p.destination_container,
                    });
                }
            }
        }
        Ok(())
    }
    fn begin_switch(
        &mut self,
        pane_id: Option<Id>,
        destination: Id,
        effects: &mut Vec<Effect>,
    ) -> Result<(), CoreError> {
        let id = self.pane_id(pane_id)?;
        self.require_idle(id)?;
        if !self.containers.contains_key(&destination) {
            return Err(CoreError::NotFound);
        }
        let p = &self.panes[&id];
        if p.container_id == destination {
            return Err(CoreError::InvalidState(
                "pane already uses the selected container".into(),
            ));
        }
        let plan = SwitchPlan {
            id: Id::new(),
            pane_id: id,
            source_container: p.container_id,
            destination_container: destination,
            pane_generation: p.generation,
            replacements: p.tabs.iter().map(replacement).collect(),
            active_source_tab: p.active_tab_id,
        };
        effects.push(Effect::RevokePane { pane_id: id });
        for r in &plan.replacements {
            effects.push(Effect::CreateTab {
                tab_id: r.replacement_tab_id,
                container_id: destination,
                url: "about:blank".into(),
            });
        }
        self.switches.insert(plan.id, plan);
        Ok(())
    }
    fn commit_switch(
        &mut self,
        id: Id,
        confirmed: bool,
        before_unload: bool,
        ready: bool,
        effects: &mut Vec<Effect>,
    ) -> Result<(), CoreError> {
        if !confirmed || !before_unload {
            return Err(CoreError::ConfirmationRequired);
        }
        if !ready {
            return Err(CoreError::ReplacementsNotReady);
        }
        let plan = self.switches.get(&id).cloned().ok_or(CoreError::NotFound)?;
        let p = self
            .panes
            .get_mut(&plan.pane_id)
            .ok_or(CoreError::StaleGeneration)?;
        if p.generation != plan.pane_generation
            || p.container_id != plan.source_container
            || p.active_tab_id != plan.active_source_tab
            || p.tabs.len() != plan.replacements.len()
            || p.tabs.iter().zip(&plan.replacements).any(|(t, r)| {
                t.id != r.source_tab_id
                    || t.navigation_generation != r.source_generation
                    || t.url != r.source_url
                    || safe_reopen_url(&t.url, t.reopen_allowed) != r.url
            })
        {
            return Err(CoreError::StaleGeneration);
        }
        effects.push(Effect::RevokePane { pane_id: p.id });
        for tab in &p.tabs {
            effects.push(Effect::CloseTab {
                tab_id: tab.id,
                container_id: p.container_id,
            });
        }
        p.container_id = plan.destination_container;
        bump(&mut p.generation)?;
        p.tabs = plan
            .replacements
            .iter()
            .map(|r| Tab::with_id(r.replacement_tab_id))
            .collect();
        p.active_tab_id = plan
            .replacements
            .iter()
            .find(|r| r.source_tab_id == plan.active_source_tab)
            .expect("validated active tab")
            .replacement_tab_id;
        self.switches.remove(&id);
        for r in plan.replacements {
            if let Some(url) = r.url {
                self.navigate(r.replacement_tab_id, url, effects)?;
            }
        }
        self.retire_unused_temporary(effects);
        Ok(())
    }
    fn begin_move(
        &mut self,
        tab: Id,
        destination: Id,
        effects: &mut Vec<Effect>,
    ) -> Result<(), CoreError> {
        let (source, index) = self.tab_location(tab).ok_or(CoreError::NotFound)?;
        self.pane_id(Some(destination))?;
        self.require_idle(source)?;
        self.require_idle(destination)?;
        let a = &self.panes[&source];
        let b = &self.panes[&destination];
        if a.container_id == b.container_id {
            return Err(CoreError::InvalidState(
                "use move_tab for same-container movement".into(),
            ));
        }
        let p = MovePlan {
            id: Id::new(),
            source_pane: source,
            destination_pane: destination,
            source_container: a.container_id,
            destination_container: b.container_id,
            source_pane_generation: a.generation,
            destination_pane_generation: b.generation,
            replacement: replacement(&a.tabs[index]),
            committed: false,
        };
        effects.push(Effect::RevokePane { pane_id: source });
        effects.push(Effect::CreateTab {
            tab_id: p.replacement.replacement_tab_id,
            container_id: b.container_id,
            url: "about:blank".into(),
        });
        self.moves.insert(p.id, p);
        Ok(())
    }
    fn commit_move(
        &mut self,
        id: Id,
        confirmed: bool,
        ready: bool,
        effects: &mut Vec<Effect>,
    ) -> Result<(), CoreError> {
        if !confirmed {
            return Err(CoreError::ConfirmationRequired);
        }
        if !ready {
            return Err(CoreError::ReplacementsNotReady);
        }
        let p = self.moves.get(&id).cloned().ok_or(CoreError::NotFound)?;
        if p.committed {
            return Err(CoreError::StaleGeneration);
        }
        let source = self
            .panes
            .get(&p.source_pane)
            .ok_or(CoreError::StaleGeneration)?;
        let dest = self
            .panes
            .get(&p.destination_pane)
            .ok_or(CoreError::StaleGeneration)?;
        let tab = source
            .tabs
            .iter()
            .find(|t| t.id == p.replacement.source_tab_id)
            .ok_or(CoreError::StaleGeneration)?;
        if source.generation != p.source_pane_generation
            || dest.generation != p.destination_pane_generation
            || source.container_id != p.source_container
            || dest.container_id != p.destination_container
            || tab.navigation_generation != p.replacement.source_generation
            || tab.url != p.replacement.source_url
            || safe_reopen_url(&tab.url, tab.reopen_allowed) != p.replacement.url
        {
            return Err(CoreError::StaleGeneration);
        }
        let dest = self
            .panes
            .get_mut(&p.destination_pane)
            .expect("known destination");
        let tab = Tab::with_id(p.replacement.replacement_tab_id);
        dest.active_tab_id = tab.id;
        dest.tabs.push(tab);
        bump(&mut dest.generation)?;
        let plan = self.moves.get_mut(&id).expect("known plan");
        plan.committed = true;
        plan.destination_pane_generation = dest.generation;
        if let Some(url) = p.replacement.url {
            self.navigate(p.replacement.replacement_tab_id, url, effects)?;
        }
        Ok(())
    }
    fn complete_move(
        &mut self,
        id: Id,
        success: bool,
        before_unload_resolved: bool,
        effects: &mut Vec<Effect>,
    ) -> Result<(), CoreError> {
        let p = self.moves.get(&id).cloned().ok_or(CoreError::NotFound)?;
        if !p.committed {
            return Err(CoreError::ReplacementsNotReady);
        }
        let destination = self
            .panes
            .get(&p.destination_pane)
            .ok_or(CoreError::StaleGeneration)?;
        let replacement = destination
            .tabs
            .iter()
            .find(|t| t.id == p.replacement.replacement_tab_id)
            .ok_or(CoreError::StaleGeneration)?;
        if destination.container_id != p.destination_container
            || replacement.navigation_generation != u64::from(p.replacement.url.is_some())
        {
            return Err(CoreError::StaleGeneration);
        }
        if success {
            if !before_unload_resolved {
                return Err(CoreError::ConfirmationRequired);
            }
            if !matches!(
                replacement.lifecycle,
                TabLifecycle::Blank | TabLifecycle::Ready
            ) {
                return Err(CoreError::ReplacementsNotReady);
            }
            let source = self
                .panes
                .get(&p.source_pane)
                .ok_or(CoreError::StaleGeneration)?;
            let tab = source
                .tabs
                .iter()
                .find(|t| t.id == p.replacement.source_tab_id)
                .ok_or(CoreError::StaleGeneration)?;
            if source.generation != p.source_pane_generation
                || source.container_id != p.source_container
                || tab.navigation_generation != p.replacement.source_generation
                || tab.url != p.replacement.source_url
            {
                return Err(CoreError::StaleGeneration);
            }
            self.close_tab(p.source_pane, p.replacement.source_tab_id, effects)?;
        } else {
            if replacement.has_before_unload || replacement.active_downloads > 0 {
                return Err(CoreError::ConfirmationRequired);
            }
            self.close_tab(
                p.destination_pane,
                p.replacement.replacement_tab_id,
                effects,
            )?;
        }
        self.moves.remove(&id);
        Ok(())
    }
}
