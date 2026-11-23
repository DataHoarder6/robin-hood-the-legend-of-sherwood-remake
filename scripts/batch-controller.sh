#!/usr/bin/env bash
# Controller for ADR 0009: agents publish readiness keyed by head SHA; one
# controller assembles a FIFO epoch into its own staging tree, preflights every
# member with git merge-tree, runs ONE tree-keyed gate, lands through land.sh's
# remote read-back, and bisects a red batch. It never hand-pushes and never
# redefines push-gate.sh / remote-check.sh / land.sh.
#
#   ready <agent> <head>   publish a compare-and-swap readiness ref + FIFO ticket
#   epoch                  cut the next batch (prints its id, tree and members)
#   gate <epoch>           run or reuse the tree-keyed gate in the staging worktree
#   land <epoch>           land through scripts/land.sh, passing the gate stamp
#   bisect <epoch>         split a red batch, land green halves, quarantine red
#   release-worthy <epoch> Conventional-Commit bump: minor|none|major-not-allowed
#
# Environment: ROBIN_BATCH_REPO, ROBIN_BATCH_STATE, ROBIN_BATCH_BASE_REF
# (fork/main), ROBIN_BATCH_MIN/MAX/WAIT_SECS, ROBIN_BATCH_GATE and
# ROBIN_BATCH_LATEST (test hooks for the gate and land commands).
set -euo pipefail

repo=$(cd -- "${ROBIN_BATCH_REPO:-.}" && pwd -P)
cd -- "$repo"
state=${ROBIN_BATCH_STATE:-${XDG_STATE_HOME:-$HOME/.local/state}/robin-batch}
base_ref=${ROBIN_BATCH_BASE_REF:-fork/main}
min_members=${ROBIN_BATCH_MIN:-4}
max_members=${ROBIN_BATCH_MAX:-5}
wait_secs=${ROBIN_BATCH_WAIT_SECS:-300}
gate_cmd=${ROBIN_BATCH_GATE:-}
land_cmd=${ROBIN_BATCH_LAND:-}

usage() {
    printf 'usage: batch-controller.sh ready|epoch|gate|land|bisect ...\n' >&2
    exit 2
}
die() { printf '%s\n' "$*"; exit 1; }

mkdir -p -- "$state/tickets" "$state/ready" "$state/parked" \
    "$state/epochs" "$state/gates" "$state/landed" "$state/quarantined" \
    "$state/worktrees"

base_tip() { git rev-parse --verify "$base_ref^{commit}"; }
meta_get() { # EPOCH KEY
    sed -n "s/^$2 //p" "$state/epochs/$1/meta" | head -1
}
head_of() { # EPOCH AGENT
    awk -v a="$2" '$1 == "member" && $2 == a { print $3; exit }' \
        "$state/epochs/$1/meta"
}
member_agents() { # EPOCH -> one agent per line
    awk '$1 == "member" { print $2 }' "$state/epochs/$1/meta"
}

# Publish readiness by CAS. A new head needs a new marker; the same head is a
# no-op. The ticket, not the ref name, carries arrival order.
ready() {
    [ $# -eq 2 ] || usage
    local agent=$1 head=$2 old base tree seq ticket line
    git rev-parse --verify -q "$head^{commit}" >/dev/null || die "not-a-commit: $head"
    head=$(git rev-parse --verify "$head^{commit}")
    base=$(base_tip) || die "missing-base-ref: $base_ref"
    tree=$(git rev-parse --verify "$head^{tree}")
    old=$(git rev-parse --verify -q "refs/robin/ready/$agent" || true)
    if [ "$old" = "$head" ]; then
        printf 'ready %s unchanged %s\n' "$agent" "$head"
        return 0
    fi
    # A re-published agent replaces its old ticket instead of jumping the queue.
    for ticket in "$state/tickets"/*; do
        [ -f "$ticket" ] || continue
        if [ "$(awk '{print $1}' "$ticket")" = "$agent" ]; then rm -f -- "$ticket"; fi
    done
    git update-ref "refs/robin/ready/$agent" "$head" "$old" ||
        die "ready-cas-failed: $agent"
    # Monotonic, never count-derived: removing an old ticket on re-publish must
    # not let an early agent reuse a number already held by a later arrival.
    seq=$(cat "$state/ticket.counter" 2>/dev/null || printf 0)
    seq=$((seq + 1))
    printf '%s\n' "$seq" >"$state/ticket.counter"
    ticket="$state/tickets/$(printf '%020d' "$seq")-$agent"
    line=$(printf '%s %s %s %s %s' "$agent" "$head" "$base" "$tree" "$(date +%s)")
    printf '%s\n' "$line" >"$ticket"
    printf '%s\n' "$line" >"$state/ready/$agent"
    printf 'ready %s ticket %s\n' "$agent" "$seq"
}

# Assemble the exact member commits onto base in a controller-only worktree.
# Returns the staging head via the caller's named variables.
assemble_into() { # NAME BASE AGENT...  (sets ASSEMBLY_HEAD/ASSEMBLY_TREE/DIR)
    local name=$1 base=$2
    shift 2
    local dir="$state/worktrees/$name"
    git worktree prune
    git worktree add --quiet -b "robin-batch/$name" "$dir" "$base" ||
        die "assembly-failed: $name"
    local agent head commit
    for agent in "$@"; do
        head=$(head_of "$CURRENT_EPOCH" "$agent")
        for commit in $(git rev-list --reverse "$base..$head"); do
            if ! git -C "$dir" cherry-pick --no-edit "$commit" >/dev/null 2>&1; then
                git -C "$dir" cherry-pick --abort >/dev/null 2>&1 || true
                die "assembly-conflict: $agent $commit"
            fi
        done
    done
    ASSEMBLY_DIR=$dir
    ASSEMBLY_HEAD=$(git -C "$dir" rev-parse HEAD)
    ASSEMBLY_TREE=$(git -C "$dir" rev-parse 'HEAD^{tree}')
}

gate_result_file() { printf '%s/gates/%s\n' "$state" "$1"; }

# One gate per tree. A green result is stamped under the tree; a replay of the
# exact tree returns it without rebuilding. Any other tree fails closed into a
# fresh gate. push-gate.sh refuses a worktree that differs from HEAD, so this
# MUST run from the controller-owned staging worktree, never the outer checkout.
run_gate() { # DIR BASE HEAD TREE
    local dir=$1 base=$2 head=$3 tree=$4 result rc=0
    result=$(gate_result_file "$tree")
    if [ -f "$result" ] && grep -qx green "$result"; then
        printf 'cached-green %s\n' "$tree"
        return 0
    fi
    if [ -n "$gate_cmd" ]; then
        ( cd -- "$dir" && "$gate_cmd" "$base" "$head" "$tree" ) || rc=$?
    else
        ( cd -- "$dir" && bash scripts/push-gate.sh "$base" "$head" ) || rc=$?
    fi
    if [ "$rc" -eq 0 ]; then
        printf 'green\n' >"$result"
        printf 'gate green %s\n' "$tree"
        return 0
    fi
    printf 'red\n' >"$result"
    printf 'gate red %s\n' "$tree"
    return 1
}

# Landing uses land.sh from the staging worktree and trusts only its read-back.
# The green stamp for THIS tree is passed through so land.sh does not gate the
# same tree a second time; a rebase changes the tree and re-enables its gate.
land_tree() { # EPOCH DIR HEAD TREE AGENT...
    local epoch=$1 dir=$2 head=$3 tree=$4
    shift 4
    local out rc=0 agent stamp
    stamp=$(gate_result_file "$tree")
    if [ -n "$land_cmd" ]; then
        out=$(cd -- "$dir" && ROBIN_LAND_GATE_PASSED_TREE="$tree" \
            ROBIN_LAND_GATE_PASSED_FILE="$stamp" "$land_cmd" 2>&1) || rc=$?
    else
        out=$(cd -- "$dir" && ROBIN_LAND_GATE_PASSED_TREE="$tree" \
            ROBIN_LAND_GATE_PASSED_FILE="$stamp" bash scripts/land.sh 2>&1) || rc=$?
    fi
    [ "$rc" -eq 0 ] || die "land-failed: $head"
    case $out in
        # land.sh proves the tip with "remote/branch = <sha>"; test stubs use
        # "readback <sha>". Either token is a read-back, push exit status is not.
        *"= $head"* | *"readback $head"*) ;;
        *) die 'land-readback-missing' ;;
    esac
    for agent in "$@"; do
        : >"$state/landed/$agent"
        git update-ref -d "refs/robin/ready/$agent" >/dev/null 2>&1 || true
    done
    printf '%s\n' "$out"
    printf 'landed %s\n' "$head"
}

epoch() {
    local -a tickets=()
    mapfile -t tickets < <(find "$state/tickets" -maxdepth 1 -type f -printf '%f\n' | sort)
    [ ${#tickets[@]} -gt 0 ] || die 'no-ready-members'
    local take=${#tickets[@]}
    [ "$take" -le "$max_members" ] || take=$max_members
    if [ "$take" -lt "$min_members" ]; then
        local oldest
        oldest=$(awk '{print $5}' "$state/tickets/${tickets[0]}")
        if [ "$(( $(date +%s) - oldest ))" -lt "$wait_secs" ]; then
            printf 'waiting %s/%s\n' "$take" "$min_members"
            exit 2
        fi
    fi

    local base epoch_no epoch_id epoch_dir agents=() pairs=()
    base=$(base_tip) || die "missing-base-ref: $base_ref"
    epoch_no=$(cat "$state/epoch.counter" 2>/dev/null || printf 0)
    epoch_no=$((epoch_no + 1))
    printf '%s\n' "$epoch_no" >"$state/epoch.counter"
    epoch_id=$(printf 'e%03d' "$epoch_no")
    epoch_dir="$state/epochs/$epoch_id"
    mkdir -p -- "$epoch_dir"
    {
        printf 'base %s\n' "$base"
        printf 'epoch %s\n' "$epoch_id"
    } >"$epoch_dir/meta"

    local i agent head at_ready tree paths rc=0 accum=$base out merged_tree
    for ((i = 0; i < take; i++)); do
        read -r agent head at_ready tree _ <"$state/tickets/${tickets[i]}"
        out=$(git merge-tree --write-tree --name-only "$accum" "$head" 2>&1) || rc=$?
        if [ "$rc" -ne 0 ]; then
            paths=$(printf '%s\n' "$out" | sed -n '2,$p' | grep -v '^$' |
                grep -vE '^(CONFLICT|Auto-merging)' | sort -u | paste -sd' ' -)
            git update-ref "refs/robin/parked/$agent" "$head"
            printf '%s %s\n' "$head" "$paths" >"$state/parked/$agent"
            printf 'parked %s: %s\n' "$agent" "$paths"
            rm -f -- "$state/tickets/${tickets[i]}"
            rc=0
            continue
        fi
        # Carry the merged result forward so a member is preflighted against
        # everyone ahead of it, not only against the base. commit-tree touches
        # no worktree or index; it is only a carrier for the accumulated tree.
        merged_tree=$(printf '%s\n' "$out" | head -1)
        accum=$(printf 'preflight\n' | git commit-tree "$merged_tree" \
            -p "$accum" -p "$head")
        agents+=("$agent")
        pairs+=("$agent $head")
        printf 'member %s %s\n' "$agent" "$head" >>"$epoch_dir/meta"
        rm -f -- "$state/tickets/${tickets[i]}"
    done
    [ ${#agents[@]} -gt 0 ] || die 'all-conflict-batch'

    CURRENT_EPOCH=$epoch_id assemble_into "$epoch_id" "$base" "${agents[@]}"
    printf 'head %s\ntree %s\nworktree %s\n' \
        "$ASSEMBLY_HEAD" "$ASSEMBLY_TREE" "$ASSEMBLY_DIR" >>"$epoch_dir/meta"
    printf 'epoch %s tree %s members %s\n' \
        "$epoch_id" "$ASSEMBLY_TREE" "${agents[*]}"
}

gate_epoch() {
    [ $# -eq 1 ] || usage
    local id=$1 tree
    [ -f "$state/epochs/$id/meta" ] || die "unknown-epoch: $id"
    tree=$(meta_get "$id" tree)
    run_gate "$(meta_get "$id" worktree)" "$(meta_get "$id" base)" \
        "$(meta_get "$id" head)" "$tree"
}

# Bisect a red batch. Green halves land; a red single member is quarantined to
# its owner. Results are keyed by each half's own tree.
bisect_epoch() {
    [ $# -eq 1 ] || usage
    local id=$1
    [ -f "$state/epochs/$id/meta" ] || die "unknown-epoch: $id"
    CURRENT_EPOCH=$id
    local base; base=$(meta_get "$id" base)
    local -a members=()
    mapfile -t members < <(member_agents "$id")
    [ ${#members[@]} -gt 0 ] || die 'empty-batch'

    local subset_no=0 landed_any=0
    bisect_group() { # AGENT...
        local -a group=("$@")
        local n=${#group[@]}
        [ "$n" -gt 0 ] || return 0
        local agent head
        if [ "$n" -eq 1 ]; then
            agent=${group[0]}
            head=$(head_of "$id" "$agent")
            subset_no=$((subset_no + 1))
            assemble_into "$id-s$subset_no" "$base" "$agent"
            if run_gate "$ASSEMBLY_DIR" "$base" "$ASSEMBLY_HEAD" "$ASSEMBLY_TREE"; then
                land_tree "$id" "$ASSEMBLY_DIR" "$ASSEMBLY_HEAD" "$ASSEMBLY_TREE" "$agent" >/dev/null
                printf 'landed-half %s\n' "$agent"
                landed_any=1
            else
                git update-ref "refs/robin/quarantined/$agent" "$head"
                printf 'quarantined %s\n' "$agent"
            fi
            return 0
        fi
        local -a left=() right=()
        local i
        for ((i = 0; i < n; i++)); do
            if [ "$i" -lt "$((n / 2))" ]; then left+=("${group[i]}"); else right+=("${group[i]}"); fi
        done
        for side in left right; do
            local -a part=()
            if [ "$side" = left ]; then part=("${left[@]}"); else part=("${right[@]}"); fi
            [ ${#part[@]} -gt 0 ] || continue
            local heads=() p
            for p in "${part[@]}"; do heads+=("$p"); done
            subset_no=$((subset_no + 1))
            assemble_into "$id-s$subset_no" "$base" "${heads[@]}"
            if run_gate "$ASSEMBLY_DIR" "$base" "$ASSEMBLY_HEAD" "$ASSEMBLY_TREE"; then
                land_tree "$id" "$ASSEMBLY_DIR" "$ASSEMBLY_HEAD" "$ASSEMBLY_TREE" "${heads[@]}" >/dev/null
                printf 'landed-half %s\n' "${heads[*]}"
                landed_any=1
            elif [ ${#heads[@]} -gt 1 ]; then
                bisect_group "${heads[@]}"
            else
                p=${heads[0]}
                git update-ref "refs/robin/quarantined/$p" "$(head_of "$id" "$p")"
                printf 'quarantined %s\n' "$p"
            fi
        done
    }
    bisect_group "${members[@]}"
    [ "$landed_any" -eq 1 ] || die 'bisect-no-landing'
}

# Release-worthy is derived from Conventional Commits in the batch, never from
# agent judgment: feat/fix -> MINOR, only docs/test/chore/... -> none, a
# breaking marker -> a human decision (major-not-allowed).
release_worthy() {
    [ $# -eq 1 ] || usage
    local id=$1 base agent head
    [ -f "$state/epochs/$id/meta" ] || die "unknown-epoch: $id"
    base=$(meta_get "$id" base)
    local -a members=()
    mapfile -t members < <(member_agents "$id")
    local text=''
    for agent in "${members[@]}"; do
        head=$(head_of "$id" "$agent")
        text+=$(git log --format='%s%n%b' "$base..$head")$'\n'
    done
    if printf '%s\n' "$text" | grep -qE '^[a-z]+(\([^)]*\))?!:|^BREAKING CHANGE:'; then
        printf 'major-not-allowed\n'
        return 1
    fi
    if printf '%s\n' "$text" | grep -qE '^(feat|fix)(\([^)]*\))?:'; then
        printf 'minor\n'
        return 0
    fi
    printf 'none\n'
}

case "${1:-}" in
    ready) shift; ready "$@" ;;
    epoch) shift; [ $# -eq 0 ] || usage; epoch ;;
    gate) shift; gate_epoch "$@" ;;
    land) shift
        [ $# -eq 1 ] || usage
        [ -f "$state/epochs/$1/meta" ] || die "unknown-epoch: $1"
        land_tree "$1" "$(meta_get "$1" worktree)" "$(meta_get "$1" head)" \
            "$(meta_get "$1" tree)" $(member_agents "$1")
        ;;
    bisect) shift; bisect_epoch "$@" ;;
    release-worthy) shift; release_worthy "$@" ;;
    *) usage ;;
esac
