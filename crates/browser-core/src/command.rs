use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    CreateContainer {
        name: String,
        persistence: Persistence,
        #[serde(default)]
        color: Option<String>,
    },
    RenameContainer {
        container_id: Id,
        name: String,
        #[serde(default)]
        color: Option<String>,
    },
    SetViewport {
        width: f64,
        height: f64,
    },
    Split {
        axis: Axis,
        #[serde(default)]
        pane_id: Option<Id>,
    },
    ClosePane {
        #[serde(default)]
        pane_id: Option<Id>,
        confirmed: bool,
    },
    FocusPane {
        pane_id: Id,
    },
    FocusDirection {
        direction: Direction,
    },
    FocusNext {
        #[serde(default)]
        backwards: bool,
    },
    FocusPrevious,
    ToggleZoom {
        #[serde(default)]
        pane_id: Option<Id>,
    },
    SwapPanes {
        first: Id,
        second: Id,
    },
    SwapAdjacent {
        #[serde(default)]
        backwards: bool,
    },
    ResizeSplit {
        split_id: Id,
        ratio: f64,
    },
    ResizeDirection {
        direction: Direction,
        pixels: f64,
    },
    NewTab {
        #[serde(default)]
        pane_id: Option<Id>,
    },
    ActivateTab {
        #[serde(default)]
        pane_id: Option<Id>,
        tab_id: Id,
    },
    CloseTab {
        #[serde(default)]
        pane_id: Option<Id>,
        #[serde(default)]
        tab_id: Option<Id>,
        confirmed: bool,
    },
    Navigate {
        #[serde(default)]
        tab_id: Option<Id>,
        url: String,
    },
    NavigationStarted {
        tab_id: Id,
        expected_generation: u64,
    },
    NavigationCommitted {
        tab_id: Id,
        expected_generation: u64,
        url: String,
        title: String,
        reopen_allowed: bool,
    },
    SetTabWarnings {
        tab_id: Id,
        has_before_unload: bool,
        active_downloads: u32,
    },
    RendererFailed {
        tab_id: Id,
        expected_generation: u64,
    },
    MoveTab {
        tab_id: Id,
        destination_pane: Id,
        #[serde(default)]
        index: Option<usize>,
    },
    BeginContainerSwitch {
        #[serde(default)]
        pane_id: Option<Id>,
        destination_container: Id,
    },
    CommitContainerSwitch {
        switch_id: Id,
        confirmed: bool,
        before_unload_resolved: bool,
        replacements_ready: bool,
    },
    CancelContainerSwitch {
        switch_id: Id,
    },
    BeginCrossContainerMove {
        tab_id: Id,
        destination_pane: Id,
    },
    CommitCrossContainerMove {
        move_id: Id,
        confirmed: bool,
        replacement_ready: bool,
    },
    CompleteCrossContainerMove {
        move_id: Id,
        success: bool,
        #[serde(default)]
        before_unload_resolved: bool,
    },
    CancelCrossContainerMove {
        move_id: Id,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Effect {
    CreateTab {
        tab_id: Id,
        container_id: Id,
        url: String,
    },
    CloseTab {
        tab_id: Id,
        container_id: Id,
    },
    Navigate {
        tab_id: Id,
        container_id: Id,
        url: String,
        navigation_generation: u64,
    },
    RevokePane {
        pane_id: Id,
    },
    RetireContainer {
        container_id: Id,
    },
    ContainerCreated {
        container_id: Id,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CommandResult {
    pub snapshot: Snapshot,
    pub effects: Vec<Effect>,
}
