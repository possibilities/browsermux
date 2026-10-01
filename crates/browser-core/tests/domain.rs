use browser_core::*;
use serde_json::{Value, json};
use std::collections::HashSet;

fn active(core: &BrowserCore) -> (Id, Id, Id) {
    let s = core.snapshot();
    let p = s
        .panes
        .iter()
        .find(|p| p.id == s.workspace.focused_pane)
        .unwrap();
    (p.id, p.active_tab_id, p.container_id)
}
fn create(core: &mut BrowserCore, name: &str, persistence: Persistence) -> Id {
    let result = core
        .dispatch(Command::CreateContainer {
            name: name.into(),
            persistence,
            color: Some("#A0B1C2".into()),
        })
        .unwrap();
    result
        .effects
        .iter()
        .find_map(|e| match e {
            Effect::ContainerCreated { container_id } => Some(*container_id),
            _ => None,
        })
        .unwrap()
}
fn switch(core: &mut BrowserCore, pane: Id, container: Id) {
    let result = core
        .dispatch(Command::BeginContainerSwitch {
            pane_id: Some(pane),
            destination_container: container,
        })
        .unwrap();
    let id = result.snapshot.pending_switches.last().unwrap().id;
    core.dispatch(Command::CommitContainerSwitch {
        switch_id: id,
        confirmed: true,
        before_unload_resolved: true,
        replacements_ready: true,
    })
    .unwrap();
}
fn commit_url(core: &mut BrowserCore, tab: Id, url: &str, reopen: bool) {
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
        title: "A page title".into(),
        reopen_allowed: reopen,
    })
    .unwrap();
}
fn split(core: &mut BrowserCore, axis: Axis) -> Id {
    core.dispatch(Command::Split {
        axis,
        pane_id: None,
    })
    .unwrap()
    .snapshot
    .workspace
    .focused_pane
}
fn assert_atomic_error(core: &mut BrowserCore, command: Command, code: &str) {
    let before = core.snapshot();
    let error = core.dispatch(command).unwrap_err();
    assert_eq!(error.code(), code);
    assert_eq!(core.snapshot(), before);
}

#[test]
fn first_run_is_blank_temporary_with_stable_explicit_identity() {
    let c = BrowserCore::new();
    let s = c.snapshot();
    assert_eq!(s.panes.len(), 1);
    assert_eq!(s.containers[0].persistence, Persistence::Temporary);
    assert_eq!(s.panes[0].tabs[0].url, "about:blank");
    assert_eq!(
        s.containers[0].session_key,
        format!("container:{}", s.containers[0].id)
    );
    assert_eq!(s.containers[0].storage_locator, None);
    c.validate().unwrap();
}
#[test]
fn nested_splits_keep_original_tab_and_container() {
    let mut c = BrowserCore::new();
    c.dispatch(Command::SetViewport {
        width: 3000.0,
        height: 2000.0,
    })
    .unwrap();
    let (p, t, container) = active(&c);
    let mut ids = HashSet::from([p]);
    for axis in [
        Axis::LeftRight,
        Axis::TopBottom,
        Axis::LeftRight,
        Axis::TopBottom,
    ] {
        ids.insert(split(&mut c, axis));
    }
    let s = c.snapshot();
    assert_eq!(s.panes.len(), 5);
    assert_eq!(
        s.workspace
            .root
            .leaf_ids()
            .into_iter()
            .collect::<HashSet<_>>(),
        ids
    );
    assert!(s.panes.iter().all(|p| p.container_id == container));
    assert_eq!(s.panes.iter().find(|x| x.id == p).unwrap().tabs[0].id, t);
    assert!(
        s.panes
            .iter()
            .flat_map(|p| &p.tabs)
            .all(|t| t.url == "about:blank")
    );
}
#[test]
fn split_below_minimum_is_atomic() {
    let mut c = BrowserCore::new();
    c.dispatch(Command::SetViewport {
        width: 400.0,
        height: 200.0,
    })
    .unwrap();
    assert_atomic_error(
        &mut c,
        Command::Split {
            axis: Axis::LeftRight,
            pane_id: None,
        },
        "insufficient_space",
    );
    assert_atomic_error(
        &mut c,
        Command::Split {
            axis: Axis::TopBottom,
            pane_id: None,
        },
        "insufficient_space",
    );
}
#[test]
fn collapse_preserves_unrelated_split_and_tabs() {
    let mut c = BrowserCore::new();
    let original = active(&c).0;
    let right = split(&mut c, Axis::LeftRight);
    let bottom = split(&mut c, Axis::TopBottom);
    let pre = c.snapshot();
    let right_branch = match &pre.workspace.root {
        Node::Split { second, .. } => (**second).clone(),
        _ => panic!(),
    };
    c.dispatch(Command::ClosePane {
        pane_id: Some(original),
        confirmed: true,
    })
    .unwrap();
    let s = c.snapshot();
    assert_eq!(s.workspace.root, right_branch);
    assert_eq!(s.workspace.root.leaf_ids(), vec![right, bottom]);
    assert_eq!(
        s.panes,
        pre.panes
            .into_iter()
            .filter(|p| p.id != original)
            .collect::<Vec<_>>()
    );
}
#[test]
fn close_last_pane_and_tab_create_blank_in_same_container() {
    let mut c = BrowserCore::new();
    let (p, t, container) = active(&c);
    c.dispatch(Command::CloseTab {
        pane_id: None,
        tab_id: None,
        confirmed: true,
    })
    .unwrap();
    assert_eq!(active(&c).0, p);
    assert_ne!(active(&c).1, t);
    c.dispatch(Command::ClosePane {
        pane_id: None,
        confirmed: true,
    })
    .unwrap();
    assert_ne!(active(&c).0, p);
    assert_eq!(active(&c).2, container);
    assert_eq!(c.snapshot().panes[0].tabs[0].url, "about:blank");
}
#[test]
fn close_commands_require_explicit_confirmation() {
    let mut c = BrowserCore::new();
    assert_atomic_error(
        &mut c,
        Command::ClosePane {
            pane_id: None,
            confirmed: false,
        },
        "confirmation_required",
    );
    assert_atomic_error(
        &mut c,
        Command::CloseTab {
            pane_id: None,
            tab_id: None,
            confirmed: false,
        },
        "confirmation_required",
    );
}
#[test]
fn zoom_is_nondestructive_and_focus_follows_visible_leaf() {
    let mut c = BrowserCore::new();
    let p = active(&c).0;
    let q = split(&mut c, Axis::LeftRight);
    let before = c.snapshot();
    c.dispatch(Command::ToggleZoom { pane_id: None }).unwrap();
    let z = c.snapshot();
    assert_eq!(z.workspace.root, before.workspace.root);
    assert_eq!(z.layout.iter().filter(|l| l.visible).count(), 1);
    assert_eq!(z.layout.iter().find(|l| l.visible).unwrap().pane_id, q);
    c.dispatch(Command::FocusPane { pane_id: p }).unwrap();
    assert_eq!(c.snapshot().workspace.zoomed_pane, Some(p));
    c.dispatch(Command::ToggleZoom { pane_id: None }).unwrap();
    assert_eq!(c.snapshot().layout, before.layout);
}
#[test]
fn spatial_focus_prioritizes_overlap_then_distance_then_tree_order() {
    let mut c = BrowserCore::new();
    let left = active(&c).0;
    let top = split(&mut c, Axis::LeftRight);
    let bottom = split(&mut c, Axis::TopBottom);
    c.dispatch(Command::FocusPane { pane_id: left }).unwrap();
    c.dispatch(Command::FocusDirection {
        direction: Direction::Right,
    })
    .unwrap();
    assert_eq!(active(&c).0, top);
    c.dispatch(Command::FocusDirection {
        direction: Direction::Down,
    })
    .unwrap();
    assert_eq!(active(&c).0, bottom);
    c.dispatch(Command::FocusPrevious).unwrap();
    assert_eq!(active(&c).0, top);
}
#[test]
fn swap_keeps_leaf_identity_container_and_live_tab() {
    let mut c = BrowserCore::new();
    let left = active(&c).0;
    let right = split(&mut c, Axis::LeftRight);
    let container = create(&mut c, "Persistent", Persistence::Persistent);
    switch(&mut c, right, container);
    let before = c.snapshot();
    c.dispatch(Command::SwapPanes {
        first: left,
        second: right,
    })
    .unwrap();
    let after = c.snapshot();
    assert_eq!(after.panes, before.panes);
    assert_eq!(after.workspace.root.leaf_ids(), vec![right, left]);
    assert_eq!(after.workspace.focused_pane, right);
}
#[test]
fn resize_clamps_and_smaller_viewports_keep_entire_tree() {
    let mut c = BrowserCore::new();
    split(&mut c, Axis::LeftRight);
    split(&mut c, Axis::TopBottom);
    let root = match c.snapshot().workspace.root {
        Node::Split { id, .. } => id,
        _ => panic!(),
    };
    for ratio in [-1e100, 1e100, 0.01, 0.99] {
        c.dispatch(Command::ResizeSplit {
            split_id: root,
            ratio,
        })
        .unwrap();
        for l in c.snapshot().layout {
            assert!(l.rect.width >= MIN_PANE_WIDTH - 0.001);
            assert!(l.rect.height >= MIN_PANE_HEIGHT - 0.001);
        }
    }
    let tree = c.snapshot().workspace.root;
    c.dispatch(Command::SetViewport {
        width: 50.0,
        height: 50.0,
    })
    .unwrap();
    let s = c.snapshot();
    assert_eq!(s.workspace.root, tree);
    assert!(s.content_size.width > s.viewport.width);
    assert!(s.layout.iter().all(|l| l.rect.width >= MIN_PANE_WIDTH));
}
#[test]
fn invalid_numeric_inputs_do_not_mutate() {
    let mut c = BrowserCore::new();
    for n in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1e10] {
        assert_atomic_error(
            &mut c,
            Command::SetViewport {
                width: n,
                height: 500.0,
            },
            "invalid_state",
        );
    }
    split(&mut c, Axis::LeftRight);
    assert_atomic_error(
        &mut c,
        Command::ResizeDirection {
            direction: Direction::Left,
            pixels: f64::NAN,
        },
        "invalid_state",
    );
}
#[test]
fn container_rename_does_not_change_canonical_storage() {
    let mut c = BrowserCore::new();
    let id = create(&mut c, "Work", Persistence::Persistent);
    let old = c
        .snapshot()
        .containers
        .into_iter()
        .find(|c| c.id == id)
        .unwrap();
    c.dispatch(Command::RenameContainer {
        container_id: id,
        name: "New label".into(),
        color: None,
    })
    .unwrap();
    let new = c
        .snapshot()
        .containers
        .into_iter()
        .find(|c| c.id == id)
        .unwrap();
    assert_eq!(new.id, old.id);
    assert_eq!(new.storage_locator, old.storage_locator);
    assert_eq!(new.session_key, old.session_key);
    assert_eq!(new.storage_locator, Some(format!("profiles/{id}")));
}
#[test]
fn duplicate_display_names_remain_distinct_sessions() {
    let mut c = BrowserCore::new();
    let a = create(&mut c, "Work", Persistence::Persistent);
    let b = create(&mut c, "Work", Persistence::Persistent);
    assert_ne!(a, b);
    let s = c.snapshot();
    assert_ne!(
        s.containers
            .iter()
            .find(|c| c.id == a)
            .unwrap()
            .storage_locator,
        s.containers
            .iter()
            .find(|c| c.id == b)
            .unwrap()
            .storage_locator
    );
}
#[test]
fn switch_prepare_and_cancel_never_emit_network_navigation() {
    let mut c = BrowserCore::new();
    let (p, t, _) = active(&c);
    commit_url(&mut c, t, "https://example.com/docs", true);
    let dest = create(&mut c, "Test", Persistence::Persistent);
    let before = c.snapshot().panes;
    let result = c
        .dispatch(Command::BeginContainerSwitch {
            pane_id: Some(p),
            destination_container: dest,
        })
        .unwrap();
    assert_eq!(result.snapshot.panes, before);
    assert!(
        result
            .effects
            .iter()
            .all(|e| !matches!(e, Effect::Navigate { .. }))
    );
    assert!(
        result
            .effects
            .iter()
            .filter_map(|e| match e {
                Effect::CreateTab { url, .. } => Some(url),
                _ => None,
            })
            .all(|url| url == "about:blank")
    );
    let plan = &result.snapshot.pending_switches[0];
    assert_eq!(
        plan.replacements[0].preview_origin.as_deref(),
        Some("https://example.com")
    );
    assert_eq!(plan.replacements[0].preview_path.as_deref(), Some("/docs"));
    let cancel = c
        .dispatch(Command::CancelContainerSwitch { switch_id: plan.id })
        .unwrap();
    assert_eq!(cancel.snapshot.panes, before);
    assert!(
        cancel
            .effects
            .iter()
            .all(|e| !matches!(e, Effect::Navigate { .. }))
    );
}
#[test]
fn switch_commit_requires_all_gates_and_replaces_ids() {
    let mut c = BrowserCore::new();
    let (p, t, source) = active(&c);
    commit_url(&mut c, t, "https://example.com/docs", true);
    let dest = create(&mut c, "Work", Persistence::Persistent);
    let begin = c
        .dispatch(Command::BeginContainerSwitch {
            pane_id: None,
            destination_container: dest,
        })
        .unwrap();
    let id = begin.snapshot.pending_switches[0].id;
    assert_atomic_error(
        &mut c,
        Command::CommitContainerSwitch {
            switch_id: id,
            confirmed: false,
            before_unload_resolved: true,
            replacements_ready: true,
        },
        "confirmation_required",
    );
    assert_atomic_error(
        &mut c,
        Command::CommitContainerSwitch {
            switch_id: id,
            confirmed: true,
            before_unload_resolved: false,
            replacements_ready: true,
        },
        "confirmation_required",
    );
    assert_atomic_error(
        &mut c,
        Command::CommitContainerSwitch {
            switch_id: id,
            confirmed: true,
            before_unload_resolved: true,
            replacements_ready: false,
        },
        "replacements_not_ready",
    );
    let commit = c
        .dispatch(Command::CommitContainerSwitch {
            switch_id: id,
            confirmed: true,
            before_unload_resolved: true,
            replacements_ready: true,
        })
        .unwrap();
    assert_eq!(active(&c).0, p);
    assert_eq!(active(&c).2, dest);
    assert_ne!(active(&c).1, t);
    assert!(
        commit
            .effects
            .iter()
            .any(|e| matches!(e,Effect::Navigate{url,..} if url=="https://example.com/docs"))
    );
    assert!(commit.effects.contains(&Effect::RetireContainer {
        container_id: source
    }));
}
#[test]
fn changed_navigation_invalidates_staged_switch_atomically() {
    let mut c = BrowserCore::new();
    let (_, t, _) = active(&c);
    commit_url(&mut c, t, "https://example.com/a", true);
    let dest = create(&mut c, "Work", Persistence::Persistent);
    let id = c
        .dispatch(Command::BeginContainerSwitch {
            pane_id: None,
            destination_container: dest,
        })
        .unwrap()
        .snapshot
        .pending_switches[0]
        .id;
    c.dispatch(Command::Navigate {
        tab_id: Some(t),
        url: "https://example.com/b".into(),
    })
    .unwrap();
    assert_atomic_error(
        &mut c,
        Command::CommitContainerSwitch {
            switch_id: id,
            confirmed: true,
            before_unload_resolved: true,
            replacements_ready: true,
        },
        "stale_generation",
    );
}
#[test]
fn unsafe_urls_and_nonreplayable_pages_become_blank_on_switch() {
    for (url, reopen) in [
        ("https://example.com/?token=secret", true),
        ("https://example.com/#secret", true),
        ("https://example.com/oauth/callback", true),
        ("https://example.com/reset/abc", true),
        ("https://example.com/order", false),
    ] {
        let mut c = BrowserCore::new();
        let (p, t, _) = active(&c);
        commit_url(&mut c, t, url, reopen);
        let dest = create(&mut c, "Work", Persistence::Persistent);
        switch(&mut c, p, dest);
        assert_eq!(c.snapshot().panes[0].tabs[0].url, "about:blank");
    }
}
#[test]
fn navigation_generations_reject_stale_callbacks_and_preserve_other_tabs() {
    let mut c = BrowserCore::new();
    let (_, t, _) = active(&c);
    c.dispatch(Command::NewTab { pane_id: None }).unwrap();
    let other = active(&c).1;
    c.dispatch(Command::Navigate {
        tab_id: Some(t),
        url: "https://a.example/".into(),
    })
    .unwrap();
    c.dispatch(Command::Navigate {
        tab_id: Some(t),
        url: "https://b.example/".into(),
    })
    .unwrap();
    assert_atomic_error(
        &mut c,
        Command::NavigationCommitted {
            tab_id: t,
            expected_generation: 1,
            url: "https://a.example/".into(),
            title: "stale".into(),
            reopen_allowed: true,
        },
        "stale_generation",
    );
    let s = c.snapshot();
    assert_eq!(
        s.panes[0]
            .tabs
            .iter()
            .find(|t| t.id == other)
            .unwrap()
            .navigation_generation,
        0
    );
    c.dispatch(Command::RendererFailed {
        tab_id: t,
        expected_generation: 2,
    })
    .unwrap();
    assert_eq!(
        c.snapshot().panes[0]
            .tabs
            .iter()
            .find(|candidate| candidate.id == t)
            .unwrap()
            .lifecycle,
        TabLifecycle::Crashed
    );
}
#[test]
fn same_container_tab_move_preserves_instance_and_refills_source() {
    let mut c = BrowserCore::new();
    let (source, tab, _) = active(&c);
    let dest = split(&mut c, Axis::LeftRight);
    commit_url(&mut c, tab, "https://example.com/", true);
    let original = c
        .snapshot()
        .panes
        .iter()
        .find(|p| p.id == source)
        .unwrap()
        .tabs[0]
        .clone();
    let result = c
        .dispatch(Command::MoveTab {
            tab_id: tab,
            destination_pane: dest,
            index: Some(0),
        })
        .unwrap();
    let dest = result.snapshot.panes.iter().find(|p| p.id == dest).unwrap();
    assert_eq!(dest.tabs[0], original);
    assert_eq!(dest.active_tab_id, tab);
    assert!(
        !result
            .effects
            .iter()
            .any(|e| matches!(e,Effect::CloseTab{tab_id,..} if *tab_id==tab))
    );
    assert_ne!(
        c.snapshot()
            .panes
            .iter()
            .find(|p| p.id == source)
            .unwrap()
            .tabs[0]
            .id,
        tab
    );
}
#[test]
fn invalid_move_index_is_atomic() {
    let mut c = BrowserCore::new();
    let (_, tab, _) = active(&c);
    let dest = split(&mut c, Axis::LeftRight);
    assert_atomic_error(
        &mut c,
        Command::MoveTab {
            tab_id: tab,
            destination_pane: dest,
            index: Some(999),
        },
        "invalid_state",
    );
}
#[test]
fn cross_container_move_keeps_source_until_observed_success() {
    let mut c = BrowserCore::new();
    let (source, tab, _) = active(&c);
    commit_url(&mut c, tab, "https://example.com/", true);
    let dest = split(&mut c, Axis::LeftRight);
    let container = create(&mut c, "Different", Persistence::Persistent);
    switch(&mut c, dest, container);
    assert_atomic_error(
        &mut c,
        Command::MoveTab {
            tab_id: tab,
            destination_pane: dest,
            index: None,
        },
        "cross_container_move",
    );
    let begin = c
        .dispatch(Command::BeginCrossContainerMove {
            tab_id: tab,
            destination_pane: dest,
        })
        .unwrap();
    let plan = &begin.snapshot.pending_moves[0];
    let replacement = plan.replacement.replacement_tab_id;
    let id = plan.id;
    c.dispatch(Command::CommitCrossContainerMove {
        move_id: id,
        confirmed: true,
        replacement_ready: true,
    })
    .unwrap();
    assert!(
        c.snapshot()
            .panes
            .iter()
            .find(|p| p.id == source)
            .unwrap()
            .tabs
            .iter()
            .any(|t| t.id == tab)
    );
    assert!(
        c.snapshot()
            .panes
            .iter()
            .find(|p| p.id == dest)
            .unwrap()
            .tabs
            .iter()
            .any(|t| t.id == replacement)
    );
    c.dispatch(Command::NavigationCommitted {
        tab_id: replacement,
        expected_generation: 1,
        url: "https://example.com/".into(),
        title: String::new(),
        reopen_allowed: true,
    })
    .unwrap();
    c.dispatch(Command::CompleteCrossContainerMove {
        move_id: id,
        success: true,
        before_unload_resolved: true,
    })
    .unwrap();
    assert!(
        !c.snapshot()
            .panes
            .iter()
            .flat_map(|p| &p.tabs)
            .any(|t| t.id == tab)
    );
}
#[test]
fn failed_cross_container_navigation_keeps_source() {
    let mut c = BrowserCore::new();
    let (source, tab, _) = active(&c);
    let dest = split(&mut c, Axis::LeftRight);
    let container = create(&mut c, "Other", Persistence::Persistent);
    switch(&mut c, dest, container);
    let id = c
        .dispatch(Command::BeginCrossContainerMove {
            tab_id: tab,
            destination_pane: dest,
        })
        .unwrap()
        .snapshot
        .pending_moves[0]
        .id;
    c.dispatch(Command::CommitCrossContainerMove {
        move_id: id,
        confirmed: true,
        replacement_ready: true,
    })
    .unwrap();
    c.dispatch(Command::CompleteCrossContainerMove {
        move_id: id,
        success: false,
        before_unload_resolved: false,
    })
    .unwrap();
    assert_eq!(
        c.snapshot()
            .panes
            .iter()
            .find(|p| p.id == source)
            .unwrap()
            .tabs[0]
            .id,
        tab
    );
    assert!(c.snapshot().pending_moves.is_empty());
}
#[test]
fn close_one_of_shared_temporary_panes_does_not_retire_container() {
    let mut c = BrowserCore::new();
    let container = active(&c).2;
    split(&mut c, Axis::LeftRight);
    let result = c
        .dispatch(Command::ClosePane {
            pane_id: None,
            confirmed: true,
        })
        .unwrap();
    assert!(!result.effects.contains(&Effect::RetireContainer {
        container_id: container
    }));
    assert_eq!(active(&c).2, container);
}
#[test]
fn url_validation_does_not_allow_privileged_or_credential_urls() {
    for bad in [
        "javascript:alert(1)",
        "data:text/html,x",
        "file:///etc/passwd",
        "chrome://settings",
        "https://user:pass@example.com/",
        "https://user@example.com/",
        "https://example.com\\@evil.com/",
        " https://example.com/",
        "https://example.com/\n",
        "",
    ] {
        assert!(validate_navigation(bad).is_err(), "{bad}");
    }
    for good in [
        "about:blank",
        "http://localhost:3000/",
        "https://example.com/path?test=yes#heading",
        "http://[::1]:8000/",
    ] {
        assert!(validate_navigation(good).is_ok(), "{good}");
    }
}
#[test]
fn command_json_rejects_unknown_fields_bad_ids_and_raw_snapshots() {
    let mut c = BrowserCore::new();
    for json in [
        r#"{"type":"new_tab","typo":true}"#,
        r#"{"type":"focus_pane","pane_id":"not-an-id"}"#,
        r#"{"type":"execute_javascript","code":"anything"}"#,
    ] {
        let before = c.snapshot();
        assert!(c.dispatch_json(json).is_err());
        assert_eq!(c.snapshot(), before);
    }
    assert!(BrowserCore::from_session(&serde_json::to_string(&c.snapshot()).unwrap()).is_err());
}
#[test]
fn restore_temporary_layout_uses_fresh_blank_identity_and_no_metadata() {
    let mut c = BrowserCore::new();
    let (_, tab, container) = active(&c);
    commit_url(&mut c, tab, "https://secret.example/private-page", true);
    c.dispatch(Command::RenameContainer {
        container_id: container,
        name: "Secret container label".into(),
        color: Some("#FfAa00".into()),
    })
    .unwrap();
    split(&mut c, Axis::LeftRight);
    let json = c.export_session().unwrap();
    for forbidden in [
        "secret.example",
        "private-page",
        "Secret container label",
        "#FfAa00",
        &tab.to_string(),
        &container.to_string(),
        "A page title",
    ] {
        assert!(!json.contains(forbidden), "leaked {forbidden}");
    }
    let restored = BrowserCore::from_session(&json).unwrap().snapshot();
    assert_eq!(restored.workspace.root, c.snapshot().workspace.root);
    assert!(restored.panes.iter().all(|p| p.container_id != container
        && p.tabs.len() == 1
        && p.tabs[0].url == "about:blank"));
    assert_eq!(restored.containers.len(), 2);
}
#[test]
fn persistent_safe_urls_and_ids_restore_without_titles_or_history() {
    let mut c = BrowserCore::new();
    let pane = active(&c).0;
    let container = create(&mut c, "Work", Persistence::Persistent);
    switch(&mut c, pane, container);
    let tab = active(&c).1;
    commit_url(&mut c, tab, "https://example.com/docs", true);
    let json = c.export_session().unwrap();
    assert!(!json.contains("A page title"));
    let restored = BrowserCore::from_session(&json).unwrap();
    assert_eq!(active(&restored), (pane, tab, container));
    assert_eq!(
        restored.snapshot().panes[0].tabs[0].url,
        "https://example.com/docs"
    );
    assert!(restored.snapshot().panes[0].tabs[0].title.is_empty());
}
#[test]
fn persistent_query_secret_is_never_serialized() {
    let mut c = BrowserCore::new();
    let pane = active(&c).0;
    let container = create(&mut c, "Work", Persistence::Persistent);
    switch(&mut c, pane, container);
    let tab = active(&c).1;
    commit_url(
        &mut c,
        tab,
        "https://example.com/login?secret=TOPSECRET",
        true,
    );
    let json = c.export_session().unwrap();
    assert!(!json.contains("TOPSECRET"));
    assert!(!json.contains("login"));
}
#[test]
fn adversarial_imports_reject_duplicate_ids_missing_references_and_bad_ratios() {
    let mut c = BrowserCore::new();
    split(&mut c, Axis::LeftRight);
    let original: Value = serde_json::from_str(&c.export_session().unwrap()).unwrap();
    let mut cases = Vec::new();
    let mut v = original.clone();
    v["schema_version"] = json!(999);
    cases.push(v);
    let mut v = original.clone();
    v["panes"][1]["id"] = v["panes"][0]["id"].clone();
    cases.push(v);
    let mut v = original.clone();
    v["workspace"]["root"]["ratio"] = json!(0);
    cases.push(v);
    let mut v = original.clone();
    v["workspace"]["root"]["ratio"] = json!(1.1);
    cases.push(v);
    let mut v = original.clone();
    v["workspace"]["focused_pane"] = json!(Id::new());
    cases.push(v);
    let mut v = original.clone();
    v["workspace"]["root"]["second"] = v["workspace"]["root"]["first"].clone();
    cases.push(v);
    let mut v = original.clone();
    v["panes"][0]["url"] = json!("https://secret.example/");
    cases.push(v);
    let mut v = original.clone();
    v["workspace"]["id"] = json!("00000000-0000-0000-0000-000000000000");
    cases.push(v);
    for v in cases {
        assert!(
            BrowserCore::from_session(&v.to_string()).is_err(),
            "accepted {v}"
        );
    }
}
#[test]
fn malicious_storage_paths_and_missing_container_never_fallback() {
    let mut c = BrowserCore::new();
    let pane = active(&c).0;
    let id = create(&mut c, "Work", Persistence::Persistent);
    switch(&mut c, pane, id);
    let v: Value = serde_json::from_str(&c.export_session().unwrap()).unwrap();
    let mut bad = v.clone();
    bad["containers"][0]["storage_locator"] = json!("../../another-profile");
    assert!(BrowserCore::from_session(&bad.to_string()).is_err());
    let mut bad = v.clone();
    bad["containers"] = json!([]);
    assert!(BrowserCore::from_session(&bad.to_string()).is_err());
    let mut bad = v;
    bad["panes"][0]["tabs"][0]["url"] = json!("file:///secret");
    assert!(BrowserCore::from_session(&bad.to_string()).is_err());
}

// Deterministic generated command sequences exercise composition and rejection,
// without a random dependency or flaky seed. Every failed command is atomic.
#[test]
fn property_style_generated_operations_preserve_invariants() {
    for seed in 1..=12u64 {
        let mut c = BrowserCore::new();
        c.dispatch(Command::SetViewport {
            width: 4096.0,
            height: 3072.0,
        })
        .unwrap();
        let mut rng = seed;
        for _ in 0..500 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let s = c.snapshot();
            let ids = s.workspace.root.leaf_ids();
            let picked = ids[(rng as usize >> 8) % ids.len()];
            let command = match rng % 12 {
                0 => Command::Split {
                    axis: Axis::LeftRight,
                    pane_id: Some(picked),
                },
                1 => Command::Split {
                    axis: Axis::TopBottom,
                    pane_id: Some(picked),
                },
                2 => Command::ClosePane {
                    pane_id: Some(picked),
                    confirmed: true,
                },
                3 => Command::FocusPane { pane_id: picked },
                4 => Command::ToggleZoom {
                    pane_id: Some(picked),
                },
                5 => Command::SwapPanes {
                    first: s.workspace.focused_pane,
                    second: picked,
                },
                6 => Command::FocusDirection {
                    direction: [
                        Direction::Left,
                        Direction::Right,
                        Direction::Up,
                        Direction::Down,
                    ][(rng as usize >> 16) % 4],
                },
                7 => Command::ResizeDirection {
                    direction: Direction::Left,
                    pixels: 16.0,
                },
                8 => Command::NewTab {
                    pane_id: Some(picked),
                },
                9 => Command::CloseTab {
                    pane_id: Some(picked),
                    tab_id: None,
                    confirmed: true,
                },
                10 => Command::FocusNext {
                    backwards: rng & 256 != 0,
                },
                _ => Command::FocusPrevious,
            };
            let result = c.dispatch(command);
            if result.is_err() {
                assert_eq!(c.snapshot(), s);
            }
            c.validate().unwrap();
            let snapshot = c.snapshot();
            assert_eq!(snapshot.layout.len(), snapshot.panes.len());
            assert!(snapshot.layout.iter().all(|p| p.rect.width >= 0.0
                && p.rect.height >= 0.0
                && p.rect.x.is_finite()
                && p.rect.y.is_finite()));
            if rng % 31 == 0 {
                let restored = BrowserCore::from_session(&c.export_session().unwrap()).unwrap();
                restored.validate().unwrap();
            }
        }
    }
}

#[test]
fn blank_preview_still_freezes_the_full_source_url() {
    let mut c = BrowserCore::new();
    let (_, tab, _) = active(&c);
    commit_url(&mut c, tab, "https://example.com/?secret=old", false);
    let destination = create(&mut c, "Work", Persistence::Persistent);
    let id = c
        .dispatch(Command::BeginContainerSwitch {
            pane_id: None,
            destination_container: destination,
        })
        .unwrap()
        .snapshot
        .pending_switches[0]
        .id;
    // Even an incorrectly reused callback generation cannot silently change the
    // frozen source from one unsafe URL to another unsafe URL.
    c.dispatch(Command::NavigationCommitted {
        tab_id: tab,
        expected_generation: 1,
        url: "https://example.com/?secret=new".into(),
        title: String::new(),
        reopen_allowed: false,
    })
    .unwrap();
    assert_atomic_error(
        &mut c,
        Command::CommitContainerSwitch {
            switch_id: id,
            confirmed: true,
            before_unload_resolved: true,
            replacements_ready: true,
        },
        "stale_generation",
    );
}
#[test]
fn cancelling_committed_move_preserves_both_tabs_and_releases_busy_state() {
    let mut c = BrowserCore::new();
    let (source, tab, _) = active(&c);
    let destination = split(&mut c, Axis::LeftRight);
    let container = create(&mut c, "Other", Persistence::Persistent);
    switch(&mut c, destination, container);
    let result = c
        .dispatch(Command::BeginCrossContainerMove {
            tab_id: tab,
            destination_pane: destination,
        })
        .unwrap();
    let plan = result.snapshot.pending_moves[0].clone();
    c.dispatch(Command::CommitCrossContainerMove {
        move_id: plan.id,
        confirmed: true,
        replacement_ready: true,
    })
    .unwrap();
    let before = c.snapshot().panes;
    let cancelled = c
        .dispatch(Command::CancelCrossContainerMove { move_id: plan.id })
        .unwrap();
    assert_eq!(cancelled.snapshot.panes, before);
    assert!(cancelled.effects.is_empty());
    c.dispatch(Command::NewTab {
        pane_id: Some(source),
    })
    .unwrap();
}
#[test]
fn cross_move_completion_requires_ready_destination_and_source_close_gate() {
    let mut c = BrowserCore::new();
    let (_, tab, _) = active(&c);
    commit_url(&mut c, tab, "https://example.com/docs", true);
    let destination = split(&mut c, Axis::LeftRight);
    let container = create(&mut c, "Other", Persistence::Persistent);
    switch(&mut c, destination, container);
    let result = c
        .dispatch(Command::BeginCrossContainerMove {
            tab_id: tab,
            destination_pane: destination,
        })
        .unwrap();
    let plan = result.snapshot.pending_moves[0].clone();
    c.dispatch(Command::CommitCrossContainerMove {
        move_id: plan.id,
        confirmed: true,
        replacement_ready: true,
    })
    .unwrap();
    assert_atomic_error(
        &mut c,
        Command::CompleteCrossContainerMove {
            move_id: plan.id,
            success: true,
            before_unload_resolved: false,
        },
        "confirmation_required",
    );
    assert_atomic_error(
        &mut c,
        Command::CompleteCrossContainerMove {
            move_id: plan.id,
            success: true,
            before_unload_resolved: true,
        },
        "replacements_not_ready",
    );
    c.dispatch(Command::NavigationCommitted {
        tab_id: plan.replacement.replacement_tab_id,
        expected_generation: 1,
        url: "https://example.com/docs".into(),
        title: String::new(),
        reopen_allowed: true,
    })
    .unwrap();
    c.dispatch(Command::CompleteCrossContainerMove {
        move_id: plan.id,
        success: true,
        before_unload_resolved: true,
    })
    .unwrap();
}
#[test]
fn changed_destination_cannot_be_discarded_by_late_move_failure() {
    let mut c = BrowserCore::new();
    let (_, tab, _) = active(&c);
    let destination = split(&mut c, Axis::LeftRight);
    let container = create(&mut c, "Other", Persistence::Persistent);
    switch(&mut c, destination, container);
    let result = c
        .dispatch(Command::BeginCrossContainerMove {
            tab_id: tab,
            destination_pane: destination,
        })
        .unwrap();
    let plan = result.snapshot.pending_moves[0].clone();
    c.dispatch(Command::CommitCrossContainerMove {
        move_id: plan.id,
        confirmed: true,
        replacement_ready: true,
    })
    .unwrap();
    c.dispatch(Command::Navigate {
        tab_id: Some(plan.replacement.replacement_tab_id),
        url: "https://example.com/new-work".into(),
    })
    .unwrap();
    assert_atomic_error(
        &mut c,
        Command::CompleteCrossContainerMove {
            move_id: plan.id,
            success: false,
            before_unload_resolved: false,
        },
        "stale_generation",
    );
    c.dispatch(Command::CancelCrossContainerMove { move_id: plan.id })
        .unwrap();
}
#[test]
fn imported_nil_split_duplicate_tab_and_active_tab_references_are_rejected() {
    let mut c = BrowserCore::new();
    let pane = active(&c).0;
    let container = create(&mut c, "Work", Persistence::Persistent);
    switch(&mut c, pane, container);
    c.dispatch(Command::NewTab { pane_id: None }).unwrap();
    split(&mut c, Axis::LeftRight);
    let original: Value = serde_json::from_str(&c.export_session().unwrap()).unwrap();
    let mut bad = original.clone();
    bad["workspace"]["root"]["id"] = json!("00000000-0000-0000-0000-000000000000");
    assert!(BrowserCore::from_session(&bad.to_string()).is_err());
    let index = original["panes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|p| p["tabs"].as_array().is_some_and(|t| t.len() == 2))
        .unwrap();
    let mut bad = original.clone();
    bad["panes"][index]["tabs"][1]["id"] = bad["panes"][index]["tabs"][0]["id"].clone();
    assert!(BrowserCore::from_session(&bad.to_string()).is_err());
    let mut bad = original;
    bad["panes"][index]["active_tab_id"] = json!(Id::new());
    assert!(BrowserCore::from_session(&bad.to_string()).is_err());
}

#[test]
fn native_isolation_fixture_fits_a_small_hosted_display() {
    let mut core = BrowserCore::new();
    core.dispatch(Command::SetViewport {
        width: 800.0,
        height: 500.0,
    })
    .unwrap();
    let original = active(&core).0;
    let a = create(&mut core, "Fixture A", Persistence::Persistent);
    let b = create(&mut core, "Fixture B", Persistence::Persistent);
    switch(&mut core, original, a);
    let right = split(&mut core, Axis::LeftRight);
    switch(&mut core, right, b);
    let bottom_right = split(&mut core, Axis::TopBottom);
    switch(&mut core, bottom_right, a);
    core.dispatch(Command::FocusPane { pane_id: original })
        .unwrap();
    let bottom_left = split(&mut core, Axis::TopBottom);
    let temp = create(&mut core, "Fixture Temp", Persistence::Temporary);
    switch(&mut core, bottom_left, temp);
    let snapshot = core.snapshot();
    assert_eq!(snapshot.panes.len(), 4);
    assert_eq!(
        snapshot
            .panes
            .iter()
            .filter(|p| p.container_id == a)
            .count(),
        2
    );
    assert_eq!(
        snapshot
            .panes
            .iter()
            .filter(|p| p.container_id == b)
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .panes
            .iter()
            .filter(|p| p.container_id == temp)
            .count(),
        1
    );
}
