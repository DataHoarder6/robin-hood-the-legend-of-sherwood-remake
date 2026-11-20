#!/usr/bin/env bash
# Print the colon-separated absolute paths of every existing build copy root.
# sccache hashes absolute paths unless basedirs are normalized; each copy must
# be listed, not just their shared parent, for a hit to cross copies.
set -euo pipefail

roots=()
for dir in "$HOME"/build/robin/*/; do
    [[ -d $dir ]] || continue
    roots+=("${dir%/}")
done

# Empty output is valid when no copy exists yet.
(IFS=:; printf '%s\n' "${roots[*]-}")
