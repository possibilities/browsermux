# Acceptance and release gates

This is a development implementation, not a finished browser alpha. A build must be evaluated at its exact commit. CI links and logs establish only the tests they actually ran.

## Implemented and portable-testable

- Recursive binary pane geometry, minimum dimensions, directional focus, resize, zoom, stable-ID swaps and collapse
- Pane-owned tabs and canonical container identities, temporary/persistent lifetimes
- Atomic command dispatch, safe navigation parsing and navigation generations
- Staged profile replacement with immutable preview, readiness and confirmation gates
- Same-container moves and confirmed cross-container fresh navigation protocol
- Sanitized versioned SQLite snapshots, root locking, generation conflict handling and corruption preservation
- Agent policy library: per-tab read/capture grants, stale-context rejection, exclusive leases, frozen approval intents, deduplication, human takeover, container barriers and manual payment/credential gates

## Native implementation under CI verification

- AppKit thin shared row, native child Chromium surfaces, pane tab picker, container picker and centered blank-page entry
- Recursive pane geometry and draggable/accessibility-adjustable dividers
- Prefix and standard menu commands, focused navigation, DevTools, find and page zoom
- Explicit CEF contexts, temporary empty-cache contexts, profile-path checks and popup container inheritance
- Renderer failure notifications, native before-unload sheets, shutdown callback ordering
- Native save dialog and download progress; one-time camera/microphone prompts
- System semantic colors and CEF color-scheme propagation
- Four-pane same-origin storage fixture: cookies, localStorage, IndexedDB, CacheStorage and same-container sharing; service worker registration exercised

The fixture does **not** independently prove all service-worker isolation semantics, operating-system identity separation, sandbox exploit resistance, IME correctness or VoiceOver usability. Those require dedicated native evidence.

## Explicitly gated or incomplete

- Native live-page container migration and cross-container tab moves are not enabled; choose a container in a blank pane
- Persistent permission management/revocation controls are not yet exposed; unhandled requests are denied, supported media is allow-once only
- Screen capture remains disabled until OS source selection, indication and stop/revoke tests pass
- Native context menus, printing, external protocol handoff, full compatibility set, native window restoration, configurable keybindings and comprehensive a11y remain acceptance work
- Unsaved work during multi-tab close cancellation needs dedicated runtime regression testing
- Agent ACP/MCP process/IPC adapters, an explicitly selected supported agent, visible grants and action execution are not implemented or enabled; portable policy tests alone do not satisfy those phases
- Page preferences, renderer recovery, protected media, OAuth, passkeys, web editors and multi-display behavior need runtime validation
- No signed/notarized distribution, update mechanism, assigned security maintainer or security-update SLA yet
- No performance claims; cold start, memory, CPU, input latency and lifetime leak budgets are unmeasured

## Manual native matrix

Before real accounts or a release, verify:

1. Six uneven panes, all keyboard/menu focus/resize/swap/zoom/close paths, small windows and display changes
2. Two same-container windows and two isolated containers across cookies, storage, workers, permissions, popups, redirects and restart
3. Rapid focus typing, IME composition, web-editor shortcuts and VoiceOver divider operation
4. Downloads, uploads, denied permissions, before-unload cancellation and active-transfer shutdown
5. One renderer crash, main-process termination during saves, malformed session and future-schema recovery
6. Light/dark and increased contrast while pages are live; page `prefers-color-scheme`
7. Sandboxed helper process behavior and no debug endpoint; untrusted certificate remains blocked
8. Repeated 100 create-close cycles, native instance count and memory after settling
9. A local dev app, OAuth sign-in, rich editor, media page and document site

Any failed gate is fixed before enabling the next security-sensitive layer.
