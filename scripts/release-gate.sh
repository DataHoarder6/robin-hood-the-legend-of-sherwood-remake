#!/usr/bin/env bash
# Release-time gate: prove H23 in a FULL, loaded multiplayer-e2e run on the build
# host and leave the evidence the D06 ledger row needs.
#
#   scripts/release-gate.sh <candidate-commit> <release-binary-on-host>
#
# Optional environment:
#   ROBIN_BUILD_HOST           build host, default minipc
#   ROBIN_RELEASE_DATA_DIR     game root on the host, default ~/Games/robin-hood/game
#   ROBIN_RELEASE_CHECKOUT_DIR checkout on the host, default ~/build/robin/e2e
#   ROBIN_E2E_EVIDENCE         persistent evidence dir, default ~/e2e-gate/release-<short>
#
# This is called from docs/RELEASES.md step 4. It is NEVER called from
# scripts/push-gate.sh or from a landing hook: a ~15-minute build-host run
# belongs to a tag, not to a push. It runs check-quality.sh, the repo's one e2e
# runner; it does not duplicate the invocation.
set -euo pipefail

exec </dev/null

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"

usage() {
    awk 'NR > 1 && /^[^#]/{exit} NR > 1{sub(/^# ?/, ""); print}' "${BASH_SOURCE[0]}" >&2
    exit 2
}
case "${1:-}" in -h | --help) usage ;; esac
if (( $# != 2 )); then
    printf 'usage: scripts/release-gate.sh <candidate-commit> <release-binary-on-host>\n' >&2
    exit 2
fi

die() { printf 'release-gate: %s\n' "$1" >&2; exit 1; }

candidate=$(git rev-parse --verify "$1^{commit}") || die "not a commit: $1"
short=${candidate:0:12}
[[ $short =~ ^[0-9a-f]{12}$ ]] || die "candidate has no 12-hex prefix: $candidate"

# The suites compile the working tree; the tag lands the commit. A tree the two
# disagree on is evidence for a tree nobody will ship.
if ! git diff --quiet HEAD -- .; then
    die 'the worktree differs from HEAD; commit the gate and its doc first'
fi
if [ -n "$(git ls-files --others --exclude-standard)" ]; then
    die 'untracked files are in the tree; commit or remove them first'
fi
head=$(git rev-parse HEAD)
[ "$head" = "$candidate" ] || die "HEAD $head is not the candidate $candidate"

host=${ROBIN_BUILD_HOST:-minipc}
binary=$2
# Keep the `~` literal: it is the BUILD HOST's home that expands it, not the
# runtime's. `${var:-"~/..."}` quotes suppress local tilde expansion.
data_dir=${ROBIN_RELEASE_DATA_DIR:-"~/Games/robin-hood/game"}
checkout=${ROBIN_RELEASE_CHECKOUT_DIR:-"~/build/robin/e2e"}
evidence=${ROBIN_E2E_EVIDENCE:-"~/e2e-gate/release-$short"}

# These strings reach a remote shell unquoted, so keep them to a safe alphabet
# rather than trusting the caller to escape them.
for item in "$binary" "$data_dir" "$checkout" "$evidence"; do
    case $item in
        '~/'* | /*) ;;
        *) die "remote paths must be absolute or ~/-relative: '$item'" ;;
    esac
    case $item in
        *[!A-Za-z0-9._/~+-]*) die "unsupported character in remote path: '$item'" ;;
    esac
done

# One ssh for the whole host side: preflight the binary against the candidate,
# require an empty evidence directory, then run the existing suite with no
# --only so D06's H23 half is proved under the corpus's full load.
suite_output=$(ssh -o BatchMode=yes -o ConnectTimeout=8 "$host" bash -s <<EOF
set -euo pipefail
checkout=$checkout
binary=$binary
data=$data_dir
evidence=$evidence
cd -- "\$checkout"
if [ ! -x "\$binary" ]; then
    printf 'release-gate: release binary %s is missing or not executable\n' "\$binary" >&2
    exit 1
fi
version=\$("\$binary" --version)
case "\$version" in
    *$short*) printf 'release-gate: binary version %s\n' "\$version" >&2 ;;
    *) printf 'release-gate: binary version %s does not carry candidate $short\n' "\$version" >&2; exit 1 ;;
esac
if [ -d "\$evidence" ] && [ -n "\$(find "\$evidence" -mindepth 1 -maxdepth 1 -print -quit)" ]; then
    printf 'release-gate: evidence dir %s is not empty\n' "\$evidence" >&2
    exit 1
fi
mkdir -p -- "\$evidence"
export ROBIN_E2E_BINARY="\$binary"
export ROBINHOOD_DATA_DIR="\$data"
export ROBIN_E2E_EVIDENCE="\$evidence"
export ROBIN_E2E_JOBS=2
nix develop --command bash scripts/check-quality.sh multiplayer-e2e
EOF
) || die "multiplayer-e2e failed on $host; evidence dir $evidence"
printf '%s\n' "$suite_output"

# The summary is authoritative, not the suite's exit code: H23 must be observed
# in a run that selected every process row, and the runner's own verdict must be
# clean too. Fetch it to a local temp file so jq runs on this checkout's side.
summary_file=$(mktemp "${TMPDIR:-/tmp}/robin-release-gate.XXXXXX")
trap 'rm -f -- "$summary_file"' EXIT
ssh "$host" "cat -- $evidence/summary.json" >"$summary_file" \
    || die "no summary.json at $host:$evidence"

# The process rows come from the harness corpus, the same source the runner
# uses, so a future `--only` creeping into check-quality.sh cannot shrink
# `.selected` into a passing gate.
expected_rows=$(python3 -B - "$repository/scripts/validation" <<'PY'
import json
import sys

sys.path.insert(0, sys.argv[1])
import scenario_corpus

_, scenarios = scenario_corpus.load_corpus()
rows = sorted(
    identifier
    for identifier, (_, document) in scenarios.items()
    if (document.get("proves") or {}).get("layer") == "process"
)
if not rows:
    print("release-gate: no process-layer rows in the corpus", file=sys.stderr)
    raise SystemExit(1)
print(json.dumps(rows))
PY
) || die 'cannot determine the process rows from scripts/validation'

if ! jq -e --argjson expected "$expected_rows" '
        . as $summary
        | ($summary.verdict == "observed")
          and (($summary.verdicts.H23 // "") == "observed")
          and (all($expected[];
                   . as $row | (($summary.selected // []) | index($row)) != null))
    ' "$summary_file" >/dev/null
then
    printf 'release-gate: %s:%s is not a clean full run:\n' "$host" "$evidence" >&2
    jq '{verdict, h23: .verdicts.H23, selected, verdicts}' "$summary_file" >&2 || true
    die 'the release gate stays red; do not flip D06'
fi

printf 'release-gate: H23 observed in the full run; evidence: %s:%s\n' "$host" "$evidence"
