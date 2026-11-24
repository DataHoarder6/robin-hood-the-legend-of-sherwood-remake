#!/usr/bin/env bash
# Fixture tests for the release parity gate. No live host, no gh, no tag push.
set -uo pipefail
source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-parity.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
export HOME=$work/home
export GIT_CONFIG_GLOBAL=$work/gitconfig
export GIT_CONFIG_NOSYSTEM=1
mkdir -p "$HOME"
git config --file "$GIT_CONFIG_GLOBAL" user.name Fixture
git config --file "$GIT_CONFIG_GLOBAL" user.email fixture@example.invalid
git config --file "$GIT_CONFIG_GLOBAL" commit.gpgsign false

repo=$work/repo
git init -q "$repo"
printf 'fixture\n' >"$repo/README.md"
git -C "$repo" add -- README.md
git -C "$repo" commit -qm fixture
git -C "$repo" tag -a v0.5.0 -m v0.5.0
short=$(git -C "$repo" rev-parse --short=12 v0.5.0^{commit})
commit=$(git -C "$repo" rev-parse v0.5.0^{commit})

site=$work/site
mkdir -p "$site/wasm/$short" "$site/datadirs/demo-local"
cat >"$site/wasm/$short/manifest.json" <<EOF
{"commit":"$commit","short":"$short","ticketSchema":3,"multiplayerContent":{"demo":{"url":"https://robin.jautis.lt/datadirs/demo-local/datadir.bin","sha256":"abc"}}}
EOF
printf 'not a datadir\n' >"$site/datadirs/demo-local/datadir.bin"
sha=$(sha256sum "$site/datadirs/demo-local/datadir.bin" | cut -d' ' -f1)
sed -i "s/\"sha256\":\"abc\"/\"sha256\":\"$sha\"/" "$site/wasm/$short/manifest.json"

stubs=$work/stubs
mkdir -p "$stubs"
cat >"$stubs/gh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' releases.win.json releases.linux.json \
  robinhood-remake-windows-Setup.exe robinhood-remake-windows-Portable.zip \
  robinhood-remake-0.5.0-windows-full.nupkg robinhood-remake-linux.AppImage \
  robinhood-remake-0.5.0-linux-full.nupkg
STUB
chmod +x "$stubs/gh"

fail=0
pass() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1" >&2; fail=1; }

run_gate() {
    ( cd "$repo" && PATH="$stubs:$PATH" ROBIN_SITE_ROOT="$site" ROBIN_REPO="$repo" \
        bash "$source_root/scripts/release-parity-check.sh" v0.5.0 )
}

# (a) today's minimal latest.json names the right commit but is not the manifest.
printf '{"commit":"%s","short":"%s"}\n' "$commit" "$short" >"$site/wasm/latest.json"
if run_gate >"$work/a.out" 2>&1; then bad 'a minimal latest.json passed'; else pass 'minimal latest.json fails'; fi
grep -q 'latest.json bytes differ' "$work/a.out" \
    && pass 'the stale selector is named' || { bad 'stale selector reason missing'; sed -n '1,8p' "$work/a.out" >&2; }

# (b) byte-identical manifest passes the selector check.
cp "$site/wasm/$short/manifest.json" "$site/wasm/latest.json"
if run_gate >"$work/b.out" 2>&1; then
    pass 'byte-identical latest.json passes'
else
    bad "byte-identical latest.json failed: $(tail -n 2 "$work/b.out")"
fi

(( fail == 0 )) || exit 1
printf 'release-parity: OK\n'
