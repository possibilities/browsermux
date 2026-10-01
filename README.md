# browsermux

A native, Apple Silicon macOS Chromium browser built around recursive panes and explicit per-pane browser profiles.

**Development work in progress.** This repository contains working, tested Rust domain and policy code and the native Swift/AppKit + Objective-C++ CEF integration. A passing Rust suite is not evidence of macOS browser correctness. See [acceptance status](docs/ACCEPTANCE.md) before using sensitive accounts. No notarized release is available.

## Architecture

- **Swift + AppKit**: a thin shared navigation row, native page hosts, compact split headers, keyboard routing, menus and accessibility
- **Rust**: authoritative recursive pane tree, stable identities, container lifecycle, safe URL restoration, transactional SQLite persistence and fail-closed agent policy
- **UniFFI**: owned Swift/Rust boundary, with versioned JSON commands and snapshots decoded to native types
- **Objective-C++**: one CEF lifetime owner, canonical request contexts, native Chromium views, callbacks and sandboxed helpers

There is no Electron implementation, WebKit substitute, Linux browser or remote-control listener.

## Build

The native app requires an Apple Silicon Mac, macOS 14.5+, an already configured official Xcode 16+ toolchain, CMake 3.21+, Python 3.12+ and Rust 1.95.0. The pinned CEF distribution targets macOS ARM64. The minimum supported end-user OS remains a release-gate decision.

```sh
cargo test --locked --workspace
scripts/build-macos.sh
open 'build/browsermux.app'
```

The build script fetches the pinned official CEF distribution, generates UniFFI Swift bindings, compiles native code and creates an ad-hoc-signed development bundle. It does not accept an Apple agreement, provision signing identities, notarize or distribute the app. GitHub Actions builds and tests the same source on standard macOS ARM64 runners.

On Linux, only the portable Rust suites and source-level checks are supported:

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
python3 scripts/security-audit.py
```

## Browser model

New windows start in a temporary container. A split inherits its parent's container and starts with a blank tab. Same-container panes intentionally share cookies and browser storage. Different containers use separate canonical CEF request contexts and immutable storage paths. This is a browser-storage boundary, not anonymity or a virtual machine.

Persistent workspace metadata is committed transactionally to SQLite. Temporary URLs, titles and identities are excluded from disk metadata. Recovery reopens only conservative HTTP(S) GET URLs; query-bearing, fragment-bearing, credential-bearing and known authentication URLs become blank. Persistent browser storage is separate from layout metadata.

Container selection currently works in blank panes. Live-page profile replacement is deliberately blocked in the native host until multi-tab before-unload transactions are validated. The Rust core implements and tests staged replacement and cross-container move protocols; their presence is not a claim that native migration is released.

## Keyboard

Control B enters one-command pane mode. Escape cancels; a second Control B passes through to the page.

| Key after prefix | Action |
| --- | --- |
| `%` | Split left/right |
| `"` | Split top/bottom |
| Arrows | Spatial focus |
| Option + arrow | Resize nearest matching divider |
| `o` / `;` | Next / previous focused pane |
| `z` | Zoom pane without destroying tree |
| `{` / `}` | Swap adjacent pane positions |
| `x` | Close pane |
| `?` | Help |

Command L focuses the address field, Command T opens a tab, Command W closes a tab, Command D splits left/right, Command Shift D splits top/bottom, and Command Shift F toggles focus mode. Search requests are sent only on Enter after choosing a provider.

## Security boundaries

- Chromium sandbox required; helpers initialize it before loading CEF
- No certificate bypass, no `--no-sandbox`, no raw CDP listener or privileged remote-page bridge
- Unknown permissions denied; camera/microphone use explicit one-time native prompts
- Screen capture is disabled pending OS-picker and revocation validation
- Downloads use a native save dialog; files never auto-open or auto-execute
- Profile root uses a process-lifetime exclusive lock
- No analytics or page-content logging
- Agent features remain disabled. The policy library tests grants, generations, exclusive mutation leases, approvals, revocation and uncertain outcomes; no arbitrary agent executable is launched

See [implementation limits](docs/ACCEPTANCE.md), [native architecture](docs/ARCHITECTURE.md), [dependency pins](docs/DEPENDENCIES.md) and [core command contract](crates/browser-core/CONTRACT.md).

## Visual references

[Guillermo Rauch's Mini](https://x.com/rauchg/status/2104428800134013205) is the primary visual reference: quiet shared chrome and an uncluttered blank page. [Alasdair Monk's Superlogical](https://x.com/almonk/status/2097439320076403125) informs compact nested-pane geometry. Their names, artwork and screenshots are not redistributed in this repository, and this project is not affiliated with either.

## Licensing

No license for this project's original code has been selected yet. Dependencies retain their own licenses. CEF and Chromium notices are bundled by the build script.
