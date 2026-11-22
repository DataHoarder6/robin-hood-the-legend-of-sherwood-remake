#!/usr/bin/env bash
# Print the live git state of every docs/BACKLOG.md entry, and flag drift:
#   - a [FROZEN] session with no BACKLOG entry  -> FAIL (breaks the freeze rule)
#   - an entry whose expected worktree is gone -> WARN (may be archived)
#
# Test seam: BACKLOG_DOC, BACKLOG_SESSION_DB and BACKLOG_REPO override the
# defaults so a fixture doc/DB can be checked without touching the live ones.
#   BACKLOG_DOC=fixture.md BACKLOG_SESSION_DB=fixture.db \
#     BACKLOG_REPO=$PWD bash scripts/backlog-check.sh
set -uo pipefail

REPO="${BACKLOG_REPO:-$(git rev-parse --show-toplevel 2>/dev/null || printf '%s' "$PWD")}"
DOC="${BACKLOG_DOC:-$REPO/docs/BACKLOG.md}"
DB="${BACKLOG_SESSION_DB:-$HOME/.local/share/opencode/opencode.db}"
SQLITE="${BACKLOG_SQLITE:-sqlite3}"

[[ -f "$DOC" ]] || { echo "backlog-check: no doc at $DOC" >&2; exit 1; }

# Entries are declared by an HTML comment (metadata, not prose) that follows the
# entry heading:
#   <!-- backlog: key=<slug> live=yes|no -->
declare -a KEYS=() LIVES=() NAMES=()
meta_re='^<!--[[:space:]]+backlog:[[:space:]]+key=([^[:space:]]+)[[:space:]]+live=(yes|no)[[:space:]]*-->$'
name=
while IFS= read -r line; do
  if [[ $line =~ ^##[[:space:]]+(.+) ]]; then
    name=${BASH_REMATCH[1]}
  elif [[ $line =~ $meta_re ]]; then
    KEYS+=("${BASH_REMATCH[1]}")
    LIVES+=("${BASH_REMATCH[2]}")
    NAMES+=("$name")
    name=
  fi
done < "$DOC"

(( ${#KEYS[@]} )) || { echo "backlog-check: no entries in $DOC" >&2; exit 1; }

fails=0
warns=0

echo "=== Backlog entries (live state from git) ==="
for i in "${!KEYS[@]}"; do
  key=${KEYS[$i]}
  live=${LIVES[$i]}
  echo "-- ${NAMES[$i]} (key=$key)"
  if [[ $live == yes ]]; then
    # Exact worktree basename: a probe worktree named "<key>-probe" is not it.
    found=
    while IFS= read -r path; do
      [[ $(basename -- "$path") == "$key" ]] && { found=$path; break; }
    done < <(git -C "$REPO" worktree list --porcelain | sed -n 's/^worktree //p')
    if [[ -n $found ]]; then
      echo "   worktree: $found"
    else
      echo "   WARN: no worktree for '$key' (archived, or never frozen)"
      warns=$((warns + 1))
    fi
    branches=$(git -C "$REPO" branch --list "*$key*")
    if [[ -n $branches ]]; then
      while IFS= read -r branch; do
        branch=${branch#* }
        echo "   branch: $(git -C "$REPO" log -1 --oneline "$branch" 2>/dev/null || printf '%s (no commit)' "$branch")"
      done <<< "$branches"
    else
      echo "   WARN: no branch matching '$key'"
      warns=$((warns + 1))
    fi
  else
    echo "   no live worktree expected (planning-only entry)"
  fi
done

echo "=== [FROZEN] sessions without an entry ==="
sqlite_bin=$(command -v "$SQLITE" 2>/dev/null || true)
if [[ -n $sqlite_bin && -f $DB ]]; then
  while IFS= read -r title; do
    [[ -n $title ]] || continue
    feature=${title#\[FROZEN\] }
    feature=${feature%% — backlog *}
    norm=${feature,,}
    norm=${norm//[^a-z0-9]/}
    matched=
    for name in "${NAMES[@]}"; do
      candidate=${name,,}
      candidate=${candidate//[^a-z0-9]/}
      [[ -n $norm && $candidate == *"$norm"* ]] && { matched=$name; break; }
    done
    if [[ -n $matched ]]; then
      echo "   OK: '$title' -> '$matched'"
    else
      echo "   FAIL: '[FROZEN]' session has no BACKLOG entry: '$title'"
      fails=$((fails + 1))
    fi
  done < <("$sqlite_bin" -readonly "$DB" \
    "SELECT title FROM session_v2 WHERE title LIKE '[FROZEN]%' ORDER BY title;" 2>/dev/null || true)
else
  echo "   SKIP: sqlite3 or session DB unavailable"
fi

echo
echo "backlog-check: fails=$fails warns=$warns"
[[ $fails -eq 0 ]] || exit 1
exit 0
