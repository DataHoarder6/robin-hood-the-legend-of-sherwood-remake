# The playable release binary: `robin` plus the engine-owned core overlay.
#
# Built with the same pinned nightly and the same linker setup as the devShell.
# The output is self-contained: the ELF interpreter, RPATH and the overlay all
# resolve inside this store path, so `nix copy` to another host is enough.
{
  lib,
  rustPlatform,
  runtimeLibs,
  gitHash,
  robinVersion,
  # Build-only tools, mirroring the devShell.
  pkg-config,
  cmake,
  clang,
  wild,
  gcc,
  python3,
  curl,
  openssl,
  alsa-lib,
  dbus,
  libpulseaudio,
  udev,
  zstd,
  makeWrapper,
}:

assert lib.assertMsg (builtins.match "[0-9a-f]{40}|[0-9a-f]{64}" gitHash != null)
  "gitHash must be a full git object ID, got ${gitHash}";
rustPlatform.buildRustPackage {
  pname = "robin";
  version = robinVersion;

  src = ../.;
  # `Cargo.lock` pins two packages named `bitcode-0.6.9` (one from crates.io,
  # one from a git fork) and the same for `bitcode_derive`, which
  # `importCargoLock`'s flat vendor directory cannot represent. `cargo vendor`
  # keys vendored sources by source, so it can.
  cargoDeps = rustPlatform.fetchCargoVendor {
    pname = "robin";
    version = robinVersion;
    src = ../.;
    hash = "sha256-U9JJ/uLLXzlII6rsBeIkUkrfP6QV5DaWoaeFEGwZO0Y=";
  };

  # `--release --features desktop,multiplayer -p robin_rs --bin robin`, the
  # combination docs/WINDOWS.md documents for a local release build.
  cargoBuildFlags = [
    "-p"
    "robin_rs"
    "--bin"
    "robin"
    "--features"
    "desktop,multiplayer"
  ];

  # Tests are gated by scripts/check-quality.sh against a dev shell and real
  # game data; a package build only has to produce the binary.
  doCheck = false;

  # `build-support/robin_build.rs` derives the engine commit from
  # `git rev-parse HEAD` and the replay format rejects anything that is not 12
  # lowercase hex. A Nix build has no `.git`, so hand the build script the
  # commit the flake resolved; it is the only way `ROBIN_GIT_HASH` can be right
  # here, and a malformed value panics instead of falling back. The variable is
  # named for this input, not for the `ROBIN_GIT_COMMIT` the script emits: Cargo
  # exports a package's own build-script `rustc-env` output to the binaries it
  # runs, so a shared name would let this build's output pose as a packager's
  # commit to every test or tool it launches.
  ROBIN_PACKAGE_COMMIT = gitHash;

  # Fail the build instead of stamping "unknown" when the identity above stops
  # working (a cargo behaviour change, a moved build script).
  ROBIN_REQUIRE_BUILD_IDENTITY = "1";

  nativeBuildInputs = [
    pkg-config
    cmake
    clang
    wild
    gcc
    python3
    curl
    makeWrapper
  ];
  # System libraries the crates probe or link: zstd-sys and libudev-sys go
  # through pkg-config and abort the build when it finds nothing, so they must
  # be present rather than bundled.
  buildInputs = [
    openssl
    alsa-lib
    dbus
    libpulseaudio
    udev
    zstd
  ];

  # Layout:
  #   bin/robin                        wrapper (LD_LIBRARY_PATH for dlopen)
  #   libexec/robin/robin              the ELF binary
  #   libexec/robin/assets             -> ../../share/robin/assets
  #   share/robin/assets/core-datadir  validated core overlay
  #
  # `resolve_install_resource_dir` probes the working directory, then
  # `<exe dir>/assets/core-datadir`; `libexec/robin/assets` makes the second
  # probe hit, so the binary needs no cwd and no asset environment variable.
  installPhase = ''
    runHook preInstall
    # `cargoBuildHook` passes `--target <triple>`, so the binary is one level
    # below `target/`; the glob fails loudly if that ever stops being true.
    install -Dm755 target/*/release/robin $out/libexec/robin/robin
    install -d $out/share/robin/assets
    cp -r assets/core-datadir $out/share/robin/assets/core-datadir
    ln -s ../../share/robin/assets $out/libexec/robin/assets
    makeWrapper $out/libexec/robin/robin $out/bin/robin \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
    runHook postInstall
  '';

  passthru = { inherit runtimeLibs; };

  meta = {
    description = "Robin Hood: The Legend of Sherwood — Rust port";
    homepage = "https://github.com/phiresky/robin-hood-the-legend-of-sherwood-remake";
    license = lib.licenses.agpl3Plus;
    mainProgram = "robin";
    platforms = lib.platforms.linux;
  };
}
