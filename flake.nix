{
  description = "Robin Hood: The Legend of Sherwood — Rust port development shell";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAllSystems (
        pkgs:
        let
          # The binary dlopens Xlib/wayland/pulse at startup, and mkShell only
          # puts packages on PATH, so their libraries need an explicit
          # LD_LIBRARY_PATH. Without it winit dies with "Failed to load one of
          # xlib's shared libraries" the moment a window opens.
          runtimeLibs = with pkgs; [
            libx11
            libxcursor
            libxi
            libxrandr
            libxcb
            libxkbcommon
            wayland
            alsa-lib
            dbus
            libpulseaudio
            systemd
            mesa
            vulkan-loader
          ];
        in
        {
          default = pkgs.mkShell {
            packages = [
              # `build-support/robin_build.rs` shells out to `git rev-parse
              # HEAD` and falls back to the literal "unknown" when it cannot:
              # the replay format tags every recording with that hash, so a
              # shell without git silently produces replays that fail version
              # validation (27 leaderboard/replay tests, 2026-11-15).
              pkgs.git
              # rust-toolchain.toml pins the compiler, rustfmt and the wasm
              # target; rustup is what honours that pin.
              pkgs.rustup
              # `.cargo/config.toml` points the linker at
              # scripts/clang-wild-linker.sh, which links with clang+wild when
              # both exist and falls back to the (much slower) system cc when
              # they do not.
              pkgs.clang
              pkgs.wild
              pkgs.gcc
              pkgs.pkg-config
              pkgs.openssl
              pkgs.cmake
              pkgs.curl
              pkgs.python3
              pkgs.xvfb
              pkgs.ffmpeg
            ]
            ++ runtimeLibs;
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
          };
        }
      );
    };
}
