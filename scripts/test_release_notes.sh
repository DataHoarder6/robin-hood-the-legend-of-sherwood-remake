#!/usr/bin/env bash
# Prove scripts/release-notes.sh whats-new is deterministic and quotes only its
# sources (annotated tags, docs/NEW_FEATURES.md and named commits in a tag's range).
set -euo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/robin-release-notes-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

fixture=$tmp/repo
git init -q -b main "$fixture"
git -C "$fixture" config user.email test@example.invalid
git -C "$fixture" config user.name test
mkdir -p "$fixture/docs/releases"

cat >"$fixture/docs/NEW_FEATURES.md" <<'EOF'
# Post-port Features

- **First feature.** It shipped first.
  Its second line is part of the same bullet.

- **Second feature.** It shipped in the second release.

## Done
EOF
git -C "$fixture" add docs/NEW_FEATURES.md
git -C "$fixture" commit -qm root
git -C "$fixture" tag -a v0.1.0 -m 'first release'

cat >"$fixture/docs/NEW_FEATURES.md" <<'EOF'
# Post-port Features

- **First feature.** It shipped first.
  Its second line is part of the same bullet.

- **Second feature.** It shipped in the second release.

- **Third feature.** It shipped in the third release.

## Done
EOF
git -C "$fixture" commit -qam 'features: third'
git -C "$fixture" tag -a v0.2.0 -m 'second release: co-op'

printf 'camera state\n' >"$fixture/camera.txt"
git -C "$fixture" add camera.txt
git -C "$fixture" commit -qm 'camera: pan from the player view and restore it on cutscene release'
git -C "$fixture" tag -a v0.3.0 -m 'third release'

python3 -B - "$fixture" "$tmp/whats-new.json" <<'PY'
import json
import subprocess
import sys

repo, out = sys.argv[1:]
def sha(short: str) -> str:
    return subprocess.check_output(["git", "-C", repo, "rev-parse", "--short=9", short], text=True).strip()
manifest = {
    "v0.1.0": {"new_features": ["First feature."], "commits": []},
    "v0.2.0": {"new_features": ["Second feature."], "commits": [sha("HEAD~1")]},
    "v0.3.0": {"new_features": ["Third feature."], "commits": [sha("HEAD")]},
}
with open(out, "w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2)
PY

out=$tmp/RELEASE_NOTES.md
bash "$repository/scripts/release-notes.sh" whats-new --repo "$fixture" \
    --source "$tmp/whats-new.json" --out "$out" >"$tmp/first.log"
cp -- "$out" "$tmp/first.md"
bash "$repository/scripts/release-notes.sh" whats-new --repo "$fixture" \
    --source "$tmp/whats-new.json" --out "$out" >"$tmp/second.log"
cmp -- "$tmp/first.md" "$out"

python3 -B - "$out" <<'PY'
import sys

text = open(sys.argv[1], encoding="utf-8").read()
sections = {}
current = None
for line in text.splitlines():
    if line.startswith("## "):
        current = line[3:]
        sections[current] = []
    elif current:
        sections[current].append(line)
assert list(sections) == ["v0.1.0", "v0.2.0", "v0.3.0"], list(sections)
assert "first release" in "\n".join(sections["v0.1.0"])
assert "It shipped first." in "\n".join(sections["v0.1.0"])
assert "Its second line is part of the same bullet." in "\n".join(sections["v0.1.0"])
assert "Second feature." not in "\n".join(sections["v0.1.0"])
assert "second release: co-op" in "\n".join(sections["v0.2.0"])
assert "Second feature." in "\n".join(sections["v0.2.0"])
assert "Third feature." not in "\n".join(sections["v0.2.0"])
assert "camera: pan from the player view and restore it on cutscene release" in "\n".join(sections["v0.3.0"])
assert "Third feature." in "\n".join(sections["v0.3.0"])
print("release notes: sources quoted verbatim, sections ordered, regeneration identical")
PY

# A commit outside the tag's range and a missing NEW_FEATURES label are errors:
# the generator must fail rather than invent a claim.
python3 -B - "$fixture" "$tmp/whats-new.json" "$tmp/bad-commit.json" "$tmp/bad-label.json" <<'PY'
import json
import sys

fixture, source, bad_commit, bad_label = sys.argv[1:]
manifest = json.load(open(source, encoding="utf-8"))
root = __import__("subprocess").check_output(
    ["git", "-C", fixture, "rev-parse", "--short=9", "HEAD~2"], text=True
).strip()
manifest["v0.3.0"]["commits"] = [root]
json.dump(manifest, open(bad_commit, "w", encoding="utf-8"))
manifest["v0.3.0"]["commits"] = []
manifest["v0.3.0"]["new_features"] = ["Never written."]
json.dump(manifest, open(bad_label, "w", encoding="utf-8"))
PY
for bad in bad-commit bad-label; do
    if bash "$repository/scripts/release-notes.sh" whats-new --repo "$fixture" \
        --source "$tmp/$bad.json" --out "$tmp/$bad.md" >"$tmp/$bad.log" 2>&1; then
        printf 'release-notes: %s should have been refused\n' "$bad" >&2
        exit 1
    fi
done
printf 'release-notes: out-of-range commit and missing feature refused\n'
