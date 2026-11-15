# Building and running on Windows

Updated 2026-11-15, for `0efa52f57`.

## TL;DR

Build with the **MSVC** toolchain on Windows:

```powershell
git clone https://github.com/DataHoarder6/robin-hood-the-legend-of-sherwood-remake
cd robin-hood-the-legend-of-sherwood-remake
cargo build --release --features desktop,multiplayer -p robin_rs --bin robin
$env:ROBINHOOD_DATA_DIR = 'C:\path\to\game'   # the GOG install dir, the one with DATA\
.\target\release\robin.exe
```

The prebuilt `windows-gnu` artifact from CI **crashes on start on some machines**;
see [The windows-gnu startup crash](#the-windows-gnu-startup-crash). The MSVC
build of the same commit is unaffected.

## Prerequisites

    winget install --id Git.Git -e
    winget install --id Rustlang.Rustup -e
    winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

`rust-toolchain.toml` pins `nightly-2026-10-21`; rustup installs it on first build.
The repo's `.cargo/config.toml` sets `codegen-backend`, which **stable cargo
rejects**, so the nightly override is not optional.

**Run cargo with the working directory inside the tree.** rustup applies
`rust-toolchain.toml` per-directory; `cargo --manifest-path C:\...\Cargo.toml`
from elsewhere silently uses the default (stable) toolchain and fails with
`the cargo feature 'codegen-backend' requires a nightly version of Cargo`.

The native helper on the link line needs **ATL**, which the `VCTools` workload
does not include. Without it the build dies at the last step with
`LNK1104: cannot open file 'atls.lib'`. Add the component:

    "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vs_installer.exe" modify `
      --installPath "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools" `
      --add Microsoft.VisualStudio.Component.VC.ATL --quiet --norestart

Drive that through `cmd /c` with one fully quoted command line. `vs_installer`
rejects `--wait` and an unquoted `--installPath` with exit code 87, and
PowerShell's `Start-Process -ArgumentList <array>` splits the path at the space.

## The windows-gnu startup crash

Symptom: the CI artifact exits instantly, even for `--version`. Windows logs
`Application Error`, `Exception code: 0xc0000005`, faulting module = the exe,
fault offset `0x3d8d376`. No window, no log line, nothing in
`%LOCALAPPDATA%\robin_hood`.

Cause: MinGW `ld` records Windows TLS `SECREL32` offsets as 32-bit base
relocations (`IMAGE_REL_BASED_HIGHLOW`) inside `.text`. The exe is linked with
`DYNAMIC_BASE`, so the loader adds the image delta to every one of them, and
every TLS read then indexes far past the thread's TLS block:

    on disk   48 8b 90 e8 09 00 00   mov rdx,[rax+0x9e8]
    in memory 48 8b 90 e8 09 4f 5c   mov rdx,[rax+0x5c4f09e8]

Measured on the failing binary: 5082 relocations inside `.text`, 4898 of 4898
`HIGHLOW` ones holding a value below the `0xB30` TLS block size — all TLS
offsets. The first such instruction to execute faults before `main`, so the
crash is independent of GPU, drivers, wgpu, Windows version, and the Velopack
stub.

Workarounds, best first:

1. **Build with MSVC** (above). No `.text` relocations of this kind are emitted.
2. **Link the gnu build without dynamicbase**, either at build time with
   `RUSTFLAGS=-C link-arg=-Wl,--disable-dynamicbase`, or use binutils >= 2.44.
3. **Patch a shipped exe**: clear `IMAGE_DLLCHARACTERISTICS_DYNAMIC_BASE` and
   `HIGH_ENTROPY_VA` in the PE optional header, so the image always loads at its
   preferred base and no relocation is applied. This costs ASLR, and a Velopack
   update overwrites the patched file.

A release gate would catch this: fail the build when `.text` holds any
`HIGHLOW` relocation, and execute `robin.exe --version` on Windows before
publishing. CI currently cross-builds the artifact and never runs it.

## Version string

The main menu and loading screen show `crate::version::version_label()` —
`v<ROBIN_PACKAGE_VERSION>`, falling back to `CARGO_PKG_VERSION`. `robin --version`
prints clap's `CARGO_PKG_VERSION` instead, so the two can disagree. Set
`ROBIN_PACKAGE_VERSION` at build time to stamp both players' menus with the same
commit:

    $env:ROBIN_PACKAGE_VERSION = '0efa52f57'
    cargo build --release --features desktop,multiplayer -p robin_rs --bin robin

## Running

`ROBINHOOD_DATA_DIR` points at the GOG installation (the directory containing
`DATA\`); set it per-user with
`[Environment]::SetEnvironmentVariable('ROBINHOOD_DATA_DIR','C:\Games\robin-hood\game','User')`,
or per-launch in a wrapper `.cmd`.

Clean build on an i5-8300H: roughly one hour. A re-stamp of only `robin_rs`
takes about 18 minutes.

## Long builds over SSH

Windows OpenSSH kills children of the SSH session when it disconnects, so a
build started over SSH dies with it. Run it as a scheduled task instead:

```powershell
$a  = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument '-NoProfile -ExecutionPolicy Bypass -File C:\path\build.ps1'
$pr = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType S4U -RunLevel Highest
Register-ScheduledTask -TaskName 'rh-build' -Action $a -Principal $pr -Force
Start-ScheduledTask -TaskName 'rh-build'
```

Passing PowerShell commands through SSH: base64-encode UTF-16LE and use
`powershell -NoProfile -EncodedCommand <b64>`; nested quoting otherwise falls
apart. `$args` is a reserved variable and will silently mangle an argument
array.

## Linux and NixOS

- Build inside `nix-shell -p ...`, not `nix shell ...`: only the former
  populates `PKG_CONFIG_PATH`, and `libdbus-sys` panics without `dbus-1.pc`.
- The built binary needs an FHS library set: winit `dlopen`s libwayland, so a
  plain `nix-shell --run` fails with `The wayland library could not be loaded`.
  Run it through `nix run --impure nixpkgs#steam-run -- ./target/release/robin`.
