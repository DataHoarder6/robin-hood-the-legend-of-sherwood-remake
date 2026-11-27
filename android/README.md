# Android Build Notes

This directory packages the Rust NativeActivity library and operator-supplied
game assets. Linux and macOS build hosts are supported. Install JDK 17 or later,
configure the Android SDK using `ANDROID_HOME` or `android/local.properties`,
accept its licenses, and install the Rust target:

```sh
rustup target add aarch64-linux-android
android/gradlew :app:assembleDebug
```

The checked-in Gradle wrapper pins Gradle 9.4.1 and verifies the distribution
SHA-256. Its official wrapper JAR digest is recorded in `gradlew`. Gradle pins
NDK 29.0.14206865. Both the manifest minimum SDK and native clang target use
API 26 (Android 8.0+); compile/target SDK remains 35. The locked CPAL 0.18 audio
backend directly links AAudio and has no OpenSL ES fallback. The previously
advertised API 24 minimum was not buildable with that backend; Android 7.x is
not supported by this configuration. Both the manifest minimum and native
compiler use the same API value. To select an optimized native library:

```sh
android/gradlew :app:assembleDebug -PrustProfile=android-release
```

The APK signing variant is independent of the Rust optimization profile.
`buildRustAndroid` always invokes Cargo; Cargo decides which Rust inputs need
rebuilding, including vendored dependencies, shared build scripts, Git refs,
toolchains and target configuration.

To validate the native build and checked-in overlay without operator game data
or a device:

```sh
CARGO_BUILD_JOBS=1 android/gradlew --no-daemon --max-workers=1 :app:buildRustAndroid :app:validateCoreOverlay
```

This produces `target/aarch64-linux-android/android-dev/librobin_rs.so`; it is
not an APK boot test.

Generate native shipping data using the converter's default source audio and
copy its complete `Data/` closure into `android/assets/Data/`, including
`datadir.bin`, missions, RHS, terrain and audio companions. Do not substitute
the browser-only Opus package. This content is not checked in.

The retail-content-free core overlay is separately packaged from
`assets/core-datadir/`. Its exact inventory and hashes are checked before
packaging. Shipping schema comes from `wasm-www/runtime-contract.json`, generated
by compiled Rust constants; CI rejects stale generated metadata:

```sh
cargo build --locked -p robin_app --example export_runtime_contract
target/debug/examples/export_runtime_contract --check wasm-www/runtime-contract.json
```

Use `--write` in place of `--check` after an intentional schema change. An
overlay/schema mismatch requires an explicit compatible overlay update; the
packager does not silently relabel old assets.

The Android boot path reads the bundled shipping datadir and the core-overlay
manifest from APK assets. It validates every declared core file by size and
SHA-256, mounts the complete bundle at engine-overlay priority, and probes all
required VFS paths before Rust initialization can construct fonts or UI. A
missing/corrupt core asset is a fatal packaging error. Loose filesystem data
remains available for debug installs by
putting a `Data/` directory in the app external files directory, or by
setting `ROBINHOOD_DATA_DIR` before startup in a custom launcher.

Android forces the graphical main menu for the bundled demo and disables
the desktop script-RPC HTTP listener. Hosting the standalone lobby broker
with `--lobby-server` is not supported. Multiplayer clients do not use a
hard-coded server: a custom launcher may provide `ROBINHOOD_LOBBY_WS`, and
the lobby UI reports that multiplayer is unavailable when it is unset.

## APK boot evidence

`validateCoreOverlay` and `buildRustAndroid` stop at `librobin_rs.so`; they are
not an APK boot test. Two explicit Gradle tasks close that gap:

```sh
android/gradlew :app:verifyApkPackaging # host only: packaged APK + overlay hashes
android/gradlew :app:verifyApkBoot      # requires an adb-attached device
```

`verifyApkPackaging` is wired into `:app:check`. It inspects the assembled APK
for `lib/arm64-v8a/librobin_rs.so` and validates every declared core-overlay
asset by size and SHA-256. `verifyApkBoot` drives a clean install, launch, boot
poll and log-evidence probe for overlay load, mission start and touch input, and
retains `app/build/reports/apk-boot-evidence.json`.

The checker is `android/scripts/apk_boot_check.py`. Its deterministic fixtures run
without a device or APK:

```sh
python3 android/scripts/apk_boot_check.py --self-test
```

A device run is **BLOCKED** when there is no Android device/emulator reachable
through `adb` and no signed APK built with the operator's signing key. The
checker prints the named capability and exits 3; it never reports a pass from a
native-library check. Boot evidence is a device observation, not a simulation
or state-hash assertion.

## Deliberate limits

- **Operator game data.** `android/assets/Data/` is generated content and stays
  out of the tree. A core-overlay-only APK is reproducible from the repository;
  reaching a mission requires the operator to supply the full `Data/` closure.
  A reproducible *playable* APK therefore needs that operator-supplied content,
  which is a capability limit rather than a packaging defect.
- **Multiplayer lobby.** There is no default lobby in standard builds. The
  custom-launcher `ROBINHOOD_LOBBY_WS` requirement is a deliberate limit; the
  lobby reports multiplayer unavailable when it is unset.
- **On-device controls.** The device/emulator gesture check is BLOCKED here.
  Code-level fixtures prove the typed mapping, not that a finger reached the
  mission on glass.

## Touch gesture map (code-level fixtures)

`crates/robin_rs/src/input_translator.rs` defines the typed touch vocabulary and
pure mapping; `crates/robin_rs/src/game_input.rs` converts the sticky modifier
latch into the existing click modifiers; `crates/robin_rs/src/host/viewport.rs`
holds the touch-camera paths whose tests consume the finger-sized hit-target
helper in `input_translator.rs`.

| Gesture | Mapped action |
| --- | --- |
| tap | primary pointer down/up |
| long press (450 ms) | secondary pointer down/up — cancel/stop, deselect, plan cancel |
| two-finger tap (250 ms) | secondary pointer down/up |
| long drag | secondary drag start |
| latched modifier + tap | Shift edges around the primary click (add to selection) |
| quick-group HUD buttons | typed assign/recall commands for slots 1-9 |
| finger down/up | presentation-only press feedback, no command side effect |

Hit targets grow to at least 48x48 logical pixels around the widget centre from
one shared `touch_target_rect()` helper; boundary points count as hits.

The real recogniser (`touch_input.rs`), dispatch (`window.rs`) and HUD gates
remain the integration point for these mappings: a device/emulator run that
drives a mission with these gestures is the retained evidence still owed.
