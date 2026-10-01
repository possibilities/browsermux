# Browser core bridge contract

Rust API: `BrowserCore::new()`, `BrowserCore::open(data_root)`, `snapshot() -> Snapshot`, `dispatch(Command) -> Result<CommandResult, CoreError>`, `dispatch_json(&str) -> Result<CommandResult, CoreError>`, `export_session() -> Result<String, CoreError>`, `from_session(&str) -> Result<Self, CoreError>`, `flush() -> Result<(), CoreError>`. `open` owns a process-lifetime exclusive root lock and SQLite connection. `new` / imports are memory-only. No CEF pointer, browsing engine, page contents, credentials, agent runtime, or network client is implemented here.

All IDs are UUID strings (`Id` newtype). `Snapshot` contains `revision`, `workspace`, `containers`, `panes`, `viewport`, `content_size`, `layout`, `pending_switches` and `pending_moves`. Workspace: `id`, `root`, `focused_pane`, `last_focused_pane`, `zoomed_pane`. Root is recursively `{kind:"leaf",pane_id}` or `{kind:"split",id,axis,ratio,first,second}`. Axis is `left_right` or `top_bottom`. Pane: `id`, `container_id`, `generation`, `tabs`, `active_tab_id`. Tab: `id`, `url`, `title`, `navigation_generation`, `lifecycle`, `reopen_allowed`, `has_before_unload`, `active_downloads`. Lifecycle: `blank`, `loading`, `ready`, `crashed`. Container: `id`, `name`, `color`, `persistence` (`persistent` / `temporary`), `session_key` (immutable canonical key), `storage_locator` (`profiles/UUID` or null). Layout: `pane_id`, `rect:{x,y,width,height}`, `visible`. CommandResult: `snapshot`, `effects`.

Commands are internally tagged by `type`, snake_case, reject unknown fields. Optional IDs omitted/null mean focused pane or active tab where documented. Non-optional fields are required. Host executes returned effects on native/CEF threads, using explicit container and tab identities. Commands are atomic: any error leaves authoritative state unchanged.

- `create_container {name,persistence,color?}`
- `rename_container {container_id,name,color?}`
- `set_viewport {width,height}`
- `split {axis,pane_id?}` (new blank tab, inherited container)
- `close_pane {pane_id?,confirmed}` (resolve native beforeunload/transfer warnings first)
- `focus_pane {pane_id}`, `focus_direction {direction}` (`left/right/up/down`), `focus_next {backwards?}`, `focus_previous {}`
- `toggle_zoom {pane_id?}`, `swap_panes {first,second}`, `swap_adjacent {backwards?}`
- `resize_split {split_id,ratio}`, `resize_direction {direction,pixels}`
- `new_tab {pane_id?}`, `activate_tab {pane_id?,tab_id}`, `close_tab {pane_id?,tab_id?,confirmed}`
- `navigate {tab_id?,url}` increments generation, returns Navigate effect
- `navigation_committed {tab_id,expected_generation,url,title,reopen_allowed}` (only callbacks for current generation accepted; replay eligibility must be false for POST, one-time, auth, upload or otherwise unsafe destinations)
- `navigation_started {tab_id,expected_generation}` (engine-initiated main-frame navigation; returns updated generation)
- `set_tab_warnings {tab_id,has_before_unload,active_downloads}`
- `renderer_failed {tab_id,expected_generation}`
- `move_tab {tab_id,destination_pane,index?}` (same container only; preserves ID and engine instance)
- `begin_container_switch {pane_id?,destination_container}` stages immutable preview, no destination requests
- `commit_container_switch {switch_id,confirmed,before_unload_resolved,replacements_ready}` must follow successful creation of blank replacement engines returned by begin; old instances retained until this commit. Stale pane/tab generations reject commit. Return CloseTab + Navigate effects, only now may network start
- `cancel_container_switch {switch_id}` discards staged replacements
- `begin_cross_container_move {tab_id,destination_pane}` / `commit_cross_container_move {move_id,confirmed,replacement_ready}` / `cancel_cross_container_move {move_id}` stage safe reopened copy; `complete_cross_container_move {move_id,success,before_unload_resolved?}` removes source only after destination success

Effects use `type` snake_case: `create_tab {tab_id,container_id,url}` (URL always about:blank), `close_tab {tab_id,container_id}`, `navigate {tab_id,container_id,url,navigation_generation}`, `revoke_pane {pane_id}`, `retire_container {container_id}`, `container_created {container_id}`. Core never independently sends requests. Reopen targets allow only credential-free HTTP(S) with no query or fragment, additionally excluding known authentication/token paths and tabs marked ineligible. This conservative rule intentionally opens unsafe URLs as blank. Confirmation preview includes origin and path. No cookie/session/history transfer.

Session JSON is a separately versioned DTO, NOT Snapshot deserialization. It retains only persistent container metadata and safe GET tab URLs; all temporary panes become blank placeholders, with no temporary container ID/name/color/tab IDs/URL/title/history. Every import validates limits, unique global IDs, all tree references, ratio finiteness/range, container identity/canonical storage, focused/active IDs, depth and dimensions. Restore creates fresh temporary containers and tabs. SQLite saves the sanitized DTO transactionally with generation conflict detection; recovery preserves corrupt files instead of overwriting them.

`CoreError` has stable `code()` and human-readable `Display`, without echoing user URLs. Full Rust definitions in `src/model.rs`, `src/command.rs`.

## Native ownership requirements and verified limits

- Returned effects are requested work, never evidence that CEF completed it. `replacements_ready` may become true only after every prepared blank browser has actually been created in its explicit destination context. If any creation fails, cancel preparation; retain the source instances.
- Closing is asynchronous. A `retire_container` effect can follow `close_tab` in the same result; keep its engine context alive until all relevant `OnBeforeClose` callbacks have completed. The root writer lock must outlive every browser and CEF shutdown.
- Navigation callbacks must carry the epoch captured at their native source. Replacing it with the model's current generation defeats stale-event rejection. Reset generations after restart are safe only because all agent grants/operations are discarded and a fresh process authentication boundary is used.
- Container-switch previews freeze the complete volatile source URL, generation and account identity, including when the policy displays a blank replacement. No plan, preview, source URL or agent lease enters a saved-session DTO.
- Cross-container move completion now accepts `before_unload_resolved` (default false). Success requires a ready destination and this final source-close gate. A late failure cannot close a destination that has navigated again or acquired warnings. Cancellation after commit preserves both tabs and ends the operation; it cannot undo destination network traffic.
- `safe_reopen_url` is conservative syntax filtering plus a host-supplied replay eligibility signal, not proof that every GET is harmless. Hosts must mark POST results, auth/one-time links, uploads and uncertain requests ineligible. Persistent restore uses only these sanitized URLs and never copies a request body or referrer.
- Single-writer locks are advisory OS locks, not a defense against a compromised user account. SQLite metadata is not an encrypted credential vault. Engine-managed disk traces and user downloads are outside temporary metadata guarantees.
- Version 1 has no speculative forward migration. A newer database or session version is preserved and refused. Structural corruption can open a blank recovery workspace while preserving original files; transient I/O or write-contention errors are returned without rotating a valid store.
- Current bounds are defensive limits: 512 panes, 4,096 tabs/containers, depth 64, 16 KiB URLs, 4 KiB titles and 8 MiB session imports. These are explicit errors, never silent eviction.

Validation on Linux: 51 tests pass, including 6,000 deterministic generated operations, serialization adversaries, native-effect ordering, SQLite atomicity/generation conflict, single-writer exclusion, corrupt-file preservation, future-schema refusal and scans of database/WAL files for temporary URL/title/container/tab leakage. These tests do not establish CEF isolation, macOS focus, native accessibility, signed distribution or browser compatibility. Those require the native acceptance suite.

### Nonblocking persistence integration

`detach_persistence() -> Option<SessionWriter>` moves SQLite and the exclusive root lock out of the model. Retain that writer until native shutdown. While briefly holding the model mutex, call `prepare_session() -> PreparedSession`; release the mutex before `writer.save(&prepared)`. The writer is independently serialized and rejects older/equal revisions, so a queued old save cannot overwrite a newer shutdown snapshot. After successful save, call `mark_session_saved(prepared.revision())`; it clears dirty state only if the model has not changed in the meantime. In-memory models return no writer. This is tested separately from the backwards-compatible synchronous `flush()` API.
