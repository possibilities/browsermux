# Dependency and toolchain pins

- Rust 1.95.0, pinned in rust-toolchain.toml
- UniFFI 0.32.2
- CEF 154.0.32+g682c378, Chromium 154.0.8037.58, official macosarm64 minimal distribution dated 2026-09-29
- CEF archive digest and byte length pinned in cef-version.json; build fetches only cef-builds.spotifycdn.com
- Rust direct dependencies pinned exactly; all transitive versions locked in Cargo.lock
- Native build requires official Xcode SDK and CMake 3.21+; no unofficial macOS SDK or Linux substitute
- GitHub standard macos-15 ARM64 runner; runner/Xcode versions are printed in every run

CEF's provided dynamic loader and C++ wrapper are built from the exact downloaded distribution. The CEF framework is loaded at runtime, not directly linked before helper sandbox initialization. Bundled LICENSE.txt and Chromium CREDITS.html are copied into the app.

Sources: [CEF general usage](https://chromiumembedded.github.io/cef/general_usage.html), [sandbox setup](https://chromiumembedded.github.io/cef/sandbox_setup.html), [UniFFI](https://github.com/mozilla/uniffi-rs), [GitHub runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
