# condx-mobile

Condx on the phone: pair it with your Mac and approve (or reject) what's waiting for your approval.
Native Shell (Rust); the design is in Condx's `docs/design/cloud.md` (phase C4).

- `src/sync.rs`: pairing, the local-first engine (`localfirst-core`) and signed decisions.
- `src/secure.rs`: the phone's identity in the Keychain (iOS) or the Keystore (Android).
- `src/lib.rs`: the screens (pair → confirm on the Mac → approvals).
- `src/theme.rs`: the desktop app's theme (tokens, flat cards, status dots, buttons), light and dark;
  the fonts (IBM Plex Sans, JetBrains Mono, OFL) are in `assets/fonts`.

```sh
nsh run --target android               # emulator (adb reverse the Cloud's port for a local backend)
scripts/ios-sim-install.sh             # iOS simulator, with the Keychain entitlement
CONDX_DAEMON_URL=http://127.0.0.1:4799 CONDX_APP_TOKEN=… cargo test --test live -- --ignored
```

The live tests need a throwaway `condxd` linked to a throwaway Condx Cloud, never a real one.

## Building the Android APK

`scripts/android-apk.sh` builds an installable APK by hand: arm64 only (half the size), 16 KB pages
(Android 15+ warns if the library is not aligned) and signed with the debug keystore. `nsh` 0.2 cannot
pick ABIs or linker flags: they are passed through `RUSTFLAGS` and x86_64 is removed from the APK afterwards.

```sh
scripts/android-apk.sh            # → .nsh/out/condx-mobile-arm64.apk
```

It needs `nsh`, the Android SDK with the NDK and build-tools, and the debug keystore
(`~/.android/debug.keystore`); `ANDROID_HOME`, `ANDROID_NDK_HOME` and `JAVA_HOME` are taken from the
environment, with macOS defaults. Install the APK on the phone with `adb install`.

## Native Shell template notes

An app written in Rust: its UI is `src/lib.rs`, and `nsh.json` names the root component (`"root": "App"`).

```sh
nsh run                      # this machine's backend; --target ios, android, web, macos, linux or windows
nsh test                     # App.test.tsx: the root tested from TypeScript, headless; --target to run it on a platform
nsh test nsh.json --trace    # headless: what the root mounts, and the time of each commit
nsh package --target web     # or ios, android, macos, linux, windows
```

`nsh run` and `nsh package` mount the root on every platform, the browser included, without QuickJS: the app is Rust
and native code from start to end (in the browser, with NSH's page script). Rust edits rebuild and restart the app.

```sh
cargo run
```

This starts the root directly on the machine's native desktop backend: GTK 4 on Linux, AppKit on macOS, Win32 on
Windows. Install the backend's development libraries before building.

The `nsh` commands build the app's runners from the NSH crates `Cargo.toml` depends on: crates.io's at the version of
the `nsh` you run (keep the `nsh-*` dependencies at that version when you upgrade `nsh`), or an NSH checkout's by path.

## Adding a TSX entry

TSX screens can use the Rust components and modules, with hot reloading for the TSX. The crate's set,
`#[native_module("condx-mobile/native")]` in `src/lib.rs`, already exports `App`. Add an `App.tsx`:

```tsx
import { mount } from "nsh";
import { App } from "condx-mobile/native";

mount(App);
```

then run it with `nsh run App.tsx` (`tsconfig.json` is there for it and the test): a named TSX entry
runs on QuickJS, while `nsh run` alone keeps running the Rust root. The first `nsh build App.tsx` writes the TypeScript
declarations of `condx-mobile/native` into `.nsh/types`. Register more with `registry.register::<…>()` (components),
`registry.module::<…>()` and `registry.class::<…>()`.

NSH-generated files stay in `.nsh/`: runner sources in `.nsh/rust/` and default packages in `.nsh/out/`.
Cargo keeps its normal `target/` directory and respects your existing `CARGO_TARGET_DIR`.
Probado con hand_off.
