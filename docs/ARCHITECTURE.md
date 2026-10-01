# Native architecture

## Ownership

BrowserCore is authoritative. Swift decodes immutable snapshots and dispatches serialized commands over generated UniFFI bindings. It does not construct profile IDs or mutate layout independently. A mutex guards the Rust state; SQLite flushes are debounced on a private I/O queue. Native event and view operations stay on the AppKit/CEF UI thread.

The Objective-C++ adapter owns every CefBrowser and CefRequestContext. A tab's NSView is retained across focus and layout changes. Closing waits for OnBeforeClose before removing engine references. Container retirement waits for the last attached browser callback. Shutdown closes browsers, leaves the CEF loop, then destroys contexts and calls CefShutdown.

The application subclass implements CefAppProtocol and wraps sendEvent with CefScopedSendingEvent. Chromium helpers initialize CefScopedSandboxContext before loading the framework. CEF command-line overrides are disabled; no unsafe debugging flags are accepted.

## Profiles

The data root is locked by Rust before engine initialization. Persistent context paths are exactly `engine/profiles/<immutable UUID>`, contained by `root_cache_path`. Every browser receives an explicit request context; there is no global-context fallback and no context-sharing constructor. Temporary profiles use a fresh context with empty cache_path. Temporary mode does not guarantee that downloads, swap, crash reports or engine-level installation files vanish.

## Native replacement gate

The domain protocol can prepare blank replacement instances, freeze safe URL previews, cancel before navigation, and commit only with explicit confirmation/readiness. The host presently limits profile switching to blank tabs so it cannot claim transactional before-unload correctness it has not established. Multi-tab native migration must resolve warnings without partially switching accounts before this gate is removed.

## Agents

The policy crate is executable, unit-tested authorization logic, not an agent runtime. Grants identify receiving session, exact tab context, origin, account epoch, capability and expiry. Operation checks compare a freshly observed context immediately before dispatch. Mutation leases serialize a tab; revocation removes observation rights and marks possibly dispatched operations uncertain. Approval records freeze the full intent and cannot be widened by page content.

No endpoint, pairing credential or agent process is created by this build. Payment and credential intents are rejected for manual control. Choosing and validating an ACP adapter, bounded stdio MCP transport and a private authenticated per-launch IPC boundary are future gated work.

## Persistence

The SQLite session stores a versioned sanitized DTO instead of serializing the live snapshot. Temporary state never enters session payloads or previous-snapshot backups. Persistent restoration is conservative and does not replay POSTs, query secrets, privileged URLs, agent operations or form values. A corrupt store is preserved; unsupported future schemas are never silently downgraded. A missing profile cannot be reassigned to whichever account is available.
