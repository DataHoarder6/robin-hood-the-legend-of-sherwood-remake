# Issues #57/#58 — Android APK and device evidence

No APK was built and no device or emulator was booted for this work. The
checker `android/scripts/apk_boot_check.py` reports BLOCKED (exit `3`) when a
capability is missing; it never writes a PASS it did not observe. Last
verified: 2026-11-26.

## What runs on this host

```sh
python3 android/scripts/apk_boot_check.py --self-test
python3 android/scripts/apk_boot_check.py --check-apk <path/to/app.apk>
```

`--self-test` runs the deterministic fixture tests (fake adb, synthetic APK and
synthetic source tree). `--check-apk` inspects a real packaged APK on the host:
native library, manifest schema, and every declared core-overlay asset's size
and SHA-256. Neither needs a device; neither is an APK boot.

The native-library/overlay build path is documented in
[android/README.md](../../android/README.md):

```sh
CARGO_BUILD_JOBS=1 android/gradlew --no-daemon --max-workers=1 \
  :app:buildRustAndroid :app:validateCoreOverlay
android/gradlew :app:assembleDebug -PrustProfile=android-release
```

The first command produces `librobin_rs.so` and validates the checked-in
overlay; the README states explicitly that it is not an APK boot test. Retail
game data under `android/assets/Data/` is operator-supplied and not checked in.

## What needs a device — and more than a device

The checker's stage needles are grounded in source: the overlay line in
`crates/robin_rs/src/android.rs`, `mission started: ` in
`crates/robin_rs/src/observability.rs`. There is no producer for a touch
evidence line, and `init_tracing()` installs no Android logcat layer. With this
tree, `--boot` is therefore BLOCKED **before** any install or launch:

```sh
python3 android/scripts/apk_boot_check.py \
  --boot <signed-apk> --adb adb \
  --evidence /tmp/opencode/android-boot.json
```

Capabilities needed, all named by the checker's `missing_capability`:

1. an Android device or working emulator reachable through `adb`, with a signed
   APK built with the operator's signing key;
2. a packaged APK carrying the core overlay and the operator's `Data/` closure;
3. a declared log producer for the touch stage under `--source-root`, and a
   confirmed `adb logcat` bridge for the packaged build's tracing output.

Adding (3) is a Rust input-layer change, outside this slice's file ownership. A
native-library check is not a substitute for a device run: #57's acceptance
(clean install, overlay validation, main menu, bundled demo mission, touch) and
#58's gesture acceptance stay unproved until a retained `--boot` evidence file
reports `PASS` on a real device or emulator. No such file exists in this tree.
