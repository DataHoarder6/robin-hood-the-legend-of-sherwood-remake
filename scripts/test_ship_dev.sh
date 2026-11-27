#!/usr/bin/env bash
# Fixture tests for scripts/ship-dev.sh. No real ssh, build host, Nix build or
# laptop copy: remote-check, ssh and nix are stubbed and every host is recorded.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-ship-dev.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

repo=$work/repo
stubs=$work/stubs
mkdir -p "$repo/scripts" "$stubs"
cp -- "$source_root/scripts/ship-dev.sh" "$repo/scripts/ship-dev.sh"

cat >"$repo/scripts/remote-check.sh" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$REMOTE_CHECK_CALLS"
[[ ${1:-} == --sync ]] || exit 64
printf '%s\n' '~/build/robin/ship-dev-fixture'
STUB

cat >"$stubs/ssh" <<'STUB'
#!/usr/bin/env bash
host=$1
shift
printf '%s\n' "$host" >>"$SSH_CALLS"
script=$(cat)
case $script in
    *'nix build .#robin --print-out-paths'*) printf '%s\n' '/nix/store/fixture-robin' ;;
    *'./result/bin/robin --version'*) printf '%s\n' 'robin fixture+0123456789ab' ;;
    *) printf 'unexpected remote script:\n%s\n' "$script" >&2; exit 65 ;;
esac
STUB

cat >"$stubs/nix" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$NIX_CALLS"
STUB

chmod +x "$repo/scripts/remote-check.sh" "$stubs/ssh" "$stubs/nix"
export PATH="$stubs:$PATH"
export REMOTE_CHECK_CALLS=$work/remote-check.calls
export SSH_CALLS=$work/ssh.calls
export NIX_CALLS=$work/nix.calls

run_case() {
    local expected_host=$1
    shift
    : >"$REMOTE_CHECK_CALLS"
    : >"$SSH_CALLS"
    : >"$NIX_CALLS"
    env "$@" bash "$repo/scripts/ship-dev.sh" >"$work/stdout" 2>"$work/stderr"
    [[ $(cat "$work/stdout") == '/nix/store/fixture-robin' ]]
    [[ $(sort -u "$SSH_CALLS") == "$expected_host" ]]
    [[ $(wc -l <"$SSH_CALLS") -eq 2 ]]
    grep -qx -- '--sync' "$REMOTE_CHECK_CALLS"
    grep -qx "copy --from ssh://$expected_host --to ssh://laptop /nix/store/fixture-robin" "$NIX_CALLS"
    grep -q 'ship-dev: building on' "$work/stderr"
    grep -q 'robin fixture+0123456789ab' "$work/stderr"
}

run_case minipc
run_case custom-build-host ROBIN_BUILD_HOST=custom-build-host

printf 'ship-dev: OK\n'
