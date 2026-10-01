use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

pub const MIN_PANE_WIDTH: f64 = 240.0;
pub const MIN_PANE_HEIGHT: f64 = 160.0;
pub const DIVIDER_SIZE: f64 = 6.0;
pub const MAX_PANES: usize = 512;
pub const MAX_TABS: usize = 4096;
pub const MAX_TREE_DEPTH: usize = 64;
pub const MAX_URL_LENGTH: usize = 16_384;
pub const MAX_SESSION_BYTES: usize = 8 * 1024 * 1024;

/// Opaque, stable identity. All externally supplied values must parse as UUIDs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(pub Uuid);
impl Id {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}
impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::str::FromStr for Id {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persistence {
    Persistent,
    Temporary,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Container {
    pub id: Id,
    pub name: String,
    pub color: Option<String>,
    pub persistence: Persistence,
    pub session_key: String,
    pub storage_locator: Option<String>,
}
impl Container {
    pub(crate) fn new(name: String, persistence: Persistence, color: Option<String>) -> Self {
        let id = Id::new();
        Self {
            id,
            name,
            color,
            persistence,
            session_key: format!("container:{id}"),
            storage_locator: (persistence == Persistence::Persistent)
                .then(|| format!("profiles/{id}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    LeftRight,
    TopBottom,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}
impl Direction {
    pub(crate) fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::LeftRight,
            _ => Axis::TopBottom,
        }
    }
    pub(crate) fn sign(self) -> f64 {
        match self {
            Self::Left | Self::Up => -1.0,
            _ => 1.0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Leaf {
        pane_id: Id,
    },
    Split {
        id: Id,
        axis: Axis,
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}
impl Node {
    pub fn leaf_ids(&self) -> Vec<Id> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }
    fn collect_leaves(&self, out: &mut Vec<Id>) {
        match self {
            Self::Leaf { pane_id } => out.push(*pane_id),
            Self::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }
    pub fn minimum_size(&self) -> Size {
        match self {
            Self::Leaf { .. } => Size {
                width: MIN_PANE_WIDTH,
                height: MIN_PANE_HEIGHT,
            },
            Self::Split {
                axis,
                first,
                second,
                ..
            } => {
                let a = first.minimum_size();
                let b = second.minimum_size();
                match axis {
                    Axis::LeftRight => Size {
                        width: a.width + b.width + DIVIDER_SIZE,
                        height: a.height.max(b.height),
                    },
                    Axis::TopBottom => Size {
                        width: a.width.max(b.width),
                        height: a.height + b.height + DIVIDER_SIZE,
                    },
                }
            }
        }
    }
    pub(crate) fn split_leaf(&mut self, target: Id, new_pane: Id, axis: Axis) -> bool {
        match self {
            Self::Leaf { pane_id } if *pane_id == target => {
                *self = Self::Split {
                    id: Id::new(),
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Leaf { pane_id: target }),
                    second: Box::new(Self::Leaf { pane_id: new_pane }),
                };
                true
            }
            Self::Split { first, second, .. } => {
                first.split_leaf(target, new_pane, axis)
                    || second.split_leaf(target, new_pane, axis)
            }
            _ => false,
        }
    }
    pub(crate) fn remove_leaf(self, target: Id) -> Option<Node> {
        match self {
            Self::Leaf { pane_id } => (pane_id != target).then_some(Self::Leaf { pane_id }),
            Self::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => match (first.remove_leaf(target), second.remove_leaf(target)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    id,
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(node), None) | (None, Some(node)) => Some(node),
                (None, None) => None,
            },
        }
    }
    pub(crate) fn swap(&mut self, a: Id, b: Id) {
        match self {
            Self::Leaf { pane_id } => {
                if *pane_id == a {
                    *pane_id = b;
                } else if *pane_id == b {
                    *pane_id = a;
                }
            }
            Self::Split { first, second, .. } => {
                first.swap(a, b);
                second.swap(a, b);
            }
        }
    }
    pub(crate) fn split_mut(&mut self, target: Id) -> Option<&mut Node> {
        match self {
            Self::Split { id, .. } if *id == target => Some(self),
            Self::Split { first, second, .. } => {
                first.split_mut(target).or_else(|| second.split_mut(target))
            }
            _ => None,
        }
    }
    pub(crate) fn ancestors(&self, target: Id, out: &mut Vec<(Id, Axis)>) -> bool {
        match self {
            Self::Leaf { pane_id } => *pane_id == target,
            Self::Split {
                id,
                axis,
                first,
                second,
                ..
            } => {
                out.push((*id, *axis));
                if first.ancestors(target, out) || second.ancestors(target, out) {
                    true
                } else {
                    out.pop();
                    false
                }
            }
        }
    }
    pub(crate) fn validate(
        &self,
        depth: usize,
        ids: &mut HashSet<Id>,
        leaves: &mut HashSet<Id>,
    ) -> Result<(), crate::CoreError> {
        if depth > MAX_TREE_DEPTH {
            return Err(crate::CoreError::InvalidState(
                "tree depth limit exceeded".into(),
            ));
        }
        match self {
            Self::Leaf { pane_id } => {
                if !leaves.insert(*pane_id) {
                    return Err(crate::CoreError::InvalidState("duplicate pane leaf".into()));
                }
            }
            Self::Split {
                id,
                ratio,
                first,
                second,
                ..
            } => {
                if !ratio.is_finite()
                    || *ratio <= 0.0
                    || *ratio >= 1.0
                    || (!ids.insert(*id) || id.0.is_nil())
                {
                    return Err(crate::CoreError::InvalidState(
                        "invalid divider or duplicate node ID".into(),
                    ));
                }
                first.validate(depth + 1, ids, leaves)?;
                second.validate(depth + 1, ids, leaves)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}
impl Default for Size {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 800.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub(crate) fn split(self, axis: Axis, ratio: f64, a: Size, b: Size) -> (Self, Self) {
        match axis {
            Axis::LeftRight => {
                let space = (self.width - DIVIDER_SIZE).max(0.0);
                let lo = a.width.min(space);
                let hi = (space - b.width).max(lo);
                let x = (space * ratio).clamp(lo, hi);
                (
                    Self { width: x, ..self },
                    Self {
                        x: self.x + x + DIVIDER_SIZE,
                        width: (space - x).max(0.0),
                        ..self
                    },
                )
            }
            Axis::TopBottom => {
                let space = (self.height - DIVIDER_SIZE).max(0.0);
                let lo = a.height.min(space);
                let hi = (space - b.height).max(lo);
                let y = (space * ratio).clamp(lo, hi);
                (
                    Self { height: y, ..self },
                    Self {
                        y: self.y + y + DIVIDER_SIZE,
                        height: (space - y).max(0.0),
                        ..self
                    },
                )
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneLayout {
    pub pane_id: Id,
    pub rect: Rect,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabLifecycle {
    Blank,
    Loading,
    Ready,
    Crashed,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub id: Id,
    pub url: String,
    pub title: String,
    pub navigation_generation: u64,
    pub lifecycle: TabLifecycle,
    pub reopen_allowed: bool,
    pub has_before_unload: bool,
    pub active_downloads: u32,
}
impl Tab {
    pub(crate) fn blank() -> Self {
        Self::with_id(Id::new())
    }
    pub(crate) fn with_id(id: Id) -> Self {
        Self {
            id,
            url: "about:blank".into(),
            title: String::new(),
            navigation_generation: 0,
            lifecycle: TabLifecycle::Blank,
            reopen_allowed: false,
            has_before_unload: false,
            active_downloads: 0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pane {
    pub id: Id,
    pub container_id: Id,
    pub generation: u64,
    pub tabs: Vec<Tab>,
    pub active_tab_id: Id,
}
impl Pane {
    pub(crate) fn blank(container_id: Id) -> Self {
        let tab = Tab::blank();
        Self {
            id: Id::new(),
            container_id,
            generation: 0,
            active_tab_id: tab.id,
            tabs: vec![tab],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: Id,
    pub root: Node,
    pub focused_pane: Id,
    pub last_focused_pane: Option<Id>,
    pub zoomed_pane: Option<Id>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Replacement {
    pub source_tab_id: Id,
    pub source_generation: u64,
    /// Volatile frozen source address; excluded from every persistence DTO.
    pub source_url: String,
    pub replacement_tab_id: Id,
    pub url: Option<String>,
    pub preview_origin: Option<String>,
    pub preview_path: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SwitchPlan {
    pub id: Id,
    pub pane_id: Id,
    pub source_container: Id,
    pub destination_container: Id,
    pub pane_generation: u64,
    pub replacements: Vec<Replacement>,
    pub active_source_tab: Id,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MovePlan {
    pub id: Id,
    pub source_pane: Id,
    pub destination_pane: Id,
    pub source_container: Id,
    pub destination_container: Id,
    pub source_pane_generation: u64,
    pub destination_pane_generation: u64,
    pub replacement: Replacement,
    pub committed: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub revision: u64,
    pub workspace: Workspace,
    pub containers: Vec<Container>,
    pub panes: Vec<Pane>,
    pub viewport: Size,
    pub content_size: Size,
    pub layout: Vec<PaneLayout>,
    pub pending_switches: Vec<SwitchPlan>,
    pub pending_moves: Vec<MovePlan>,
}
#[derive(Clone, Debug)]
pub(crate) struct State {
    pub revision: u64,
    pub workspace: Workspace,
    pub containers: BTreeMap<Id, Container>,
    pub panes: BTreeMap<Id, Pane>,
    pub viewport: Size,
    pub switches: BTreeMap<Id, SwitchPlan>,
    pub moves: BTreeMap<Id, MovePlan>,
}
impl State {
    pub(crate) fn fresh() -> Self {
        let c = Container::new("Temporary".into(), Persistence::Temporary, None);
        let p = Pane::blank(c.id);
        Self {
            revision: 0,
            workspace: Workspace {
                id: Id::new(),
                root: Node::Leaf { pane_id: p.id },
                focused_pane: p.id,
                last_focused_pane: None,
                zoomed_pane: None,
            },
            containers: BTreeMap::from([(c.id, c)]),
            panes: BTreeMap::from([(p.id, p)]),
            viewport: Size::default(),
            switches: BTreeMap::new(),
            moves: BTreeMap::new(),
        }
    }
    pub(crate) fn content_size(&self) -> Size {
        let min = self.workspace.root.minimum_size();
        Size {
            width: self.viewport.width.max(min.width),
            height: self.viewport.height.max(min.height),
        }
    }
    pub(crate) fn base_layout(&self) -> Vec<PaneLayout> {
        let size = self.content_size();
        let mut out = Vec::new();
        walk_layout(
            &self.workspace.root,
            Rect {
                x: 0.0,
                y: 0.0,
                width: size.width,
                height: size.height,
            },
            &mut out,
        );
        out
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        let mut layout = self.base_layout();
        if let Some(zoom) = self.workspace.zoomed_pane {
            for item in &mut layout {
                item.visible = item.pane_id == zoom;
                if item.visible {
                    item.rect = Rect {
                        x: 0.0,
                        y: 0.0,
                        width: self.viewport.width,
                        height: self.viewport.height,
                    };
                }
            }
        }
        Snapshot {
            revision: self.revision,
            workspace: self.workspace.clone(),
            containers: self.containers.values().cloned().collect(),
            panes: self.panes.values().cloned().collect(),
            viewport: self.viewport,
            content_size: self.content_size(),
            layout,
            pending_switches: self.switches.values().cloned().collect(),
            pending_moves: self.moves.values().cloned().collect(),
        }
    }
    pub(crate) fn tab_location(&self, id: Id) -> Option<(Id, usize)> {
        self.panes
            .values()
            .find_map(|p| p.tabs.iter().position(|t| t.id == id).map(|i| (p.id, i)))
    }
    pub(crate) fn validate(&self) -> Result<(), crate::CoreError> {
        use crate::CoreError::InvalidState;
        if self.panes.is_empty() || self.panes.len() > MAX_PANES || self.containers.len() > MAX_TABS
        {
            return Err(InvalidState("invalid collection size".into()));
        }
        if !self.viewport.width.is_finite()
            || !self.viewport.height.is_finite()
            || self.viewport.width <= 0.0
            || self.viewport.height <= 0.0
            || self.viewport.width > 1_000_000.0
            || self.viewport.height > 1_000_000.0
        {
            return Err(InvalidState("invalid viewport".into()));
        }
        if self.workspace.id.0.is_nil() {
            return Err(InvalidState("workspace ID cannot be nil".into()));
        }
        let mut ids = HashSet::new();
        ids.insert(self.workspace.id);
        let mut leaves = HashSet::new();
        self.workspace.root.validate(0, &mut ids, &mut leaves)?;
        if leaves != self.panes.keys().copied().collect() {
            return Err(InvalidState("tree and pane registry differ".into()));
        }
        if !leaves.contains(&self.workspace.focused_pane)
            || self
                .workspace
                .last_focused_pane
                .is_some_and(|id| !leaves.contains(&id))
            || self
                .workspace
                .zoomed_pane
                .is_some_and(|id| !leaves.contains(&id))
        {
            return Err(InvalidState("dangling focus reference".into()));
        }
        for (id, c) in &self.containers {
            if *id != c.id || !ids.insert(*id) || id.0.is_nil() {
                return Err(InvalidState("duplicate container ID".into()));
            }
            crate::validate_label(&c.name)?;
            crate::validate_color(c.color.as_deref())?;
            if c.session_key != format!("container:{id}")
                || c.storage_locator
                    != (c.persistence == Persistence::Persistent).then(|| format!("profiles/{id}"))
            {
                return Err(InvalidState(
                    "noncanonical container storage identity".into(),
                ));
            }
        }
        let mut count = 0;
        for (id, p) in &self.panes {
            if *id != p.id
                || !ids.insert(*id)
                || id.0.is_nil()
                || !self.containers.contains_key(&p.container_id)
                || p.tabs.is_empty()
                || !p.tabs.iter().any(|t| t.id == p.active_tab_id)
            {
                return Err(InvalidState("invalid pane".into()));
            }
            for t in &p.tabs {
                count += 1;
                if !ids.insert(t.id)
                    || t.id.0.is_nil()
                    || t.url.len() > MAX_URL_LENGTH
                    || t.title.len() > 4096
                {
                    return Err(InvalidState("invalid or duplicate tab".into()));
                }
                crate::validate_navigation(&t.url)?;
            }
        }
        if count > MAX_TABS {
            return Err(InvalidState("tab limit exceeded".into()));
        }
        Ok(())
    }
}
fn walk_layout(node: &Node, rect: Rect, out: &mut Vec<PaneLayout>) {
    match node {
        Node::Leaf { pane_id } => out.push(PaneLayout {
            pane_id: *pane_id,
            rect,
            visible: true,
        }),
        Node::Split {
            axis,
            ratio,
            first,
            second,
            ..
        } => {
            let (a, b) = rect.split(*axis, *ratio, first.minimum_size(), second.minimum_size());
            walk_layout(first, a, out);
            walk_layout(second, b, out);
        }
    }
}
pub(crate) fn split_geometry(
    node: &Node,
    target: Id,
    rect: Rect,
) -> Option<(Rect, Axis, Size, Size)> {
    match node {
        Node::Split {
            id,
            axis,
            first,
            second,
            ratio,
        } => {
            if *id == target {
                return Some((rect, *axis, first.minimum_size(), second.minimum_size()));
            }
            let (a, b) = rect.split(*axis, *ratio, first.minimum_size(), second.minimum_size());
            split_geometry(first, target, a).or_else(|| split_geometry(second, target, b))
        }
        _ => None,
    }
}
