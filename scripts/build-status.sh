#!/usr/bin/env bash
# One-shot, read-only report of the build hosts and the local checkout.
#
#   scripts/build-status.sh
#
# Nothing here changes state: each remote section runs in a `bash -s` fed from
# stdin and reports an unreachable host instead of failing the whole command.
# Paths and hosts come from scripts/remote-check.sh (the gate's own constants):
# copies live under ~/build/robin, slots in its .slots directory, and the disk
# floor is ROBIN_MIN_FREE_GB. `homelab` is the estate's second gate host.
set -uo pipefail

repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"

remote_root=build/robin
slots=${ROBIN_BUILD_SLOTS:-3}
min_free_gb=${ROBIN_MIN_FREE_GB:-40}
hosts=("${ROBIN_BUILD_HOST:-minipc}" homelab)

for host in "${hosts[@]}"; do
    printf '\n===== %s =====\n' "$host"
    if ! ssh -o BatchMode=yes -o ConnectTimeout=8 "$host" bash -s -- \
        "$remote_root" "$slots" "$min_free_gb" <<'REMOTE'
set -u
root=$HOME/$1
slots=$2
min_gb=$3
slot_dir=$root/.slots

printf 'copies:\n'
if [[ -d $root ]]; then
    du -sh -- "$root"/*/ 2>/dev/null || printf '  (none)\n'
else
    printf '  %s does not exist\n' "$root"
fi

held=0
for ((i = 1; i <= slots; i++)); do
    [[ -f $slot_dir/$i ]] || continue
    if ! flock -n "$slot_dir/$i" true 2>/dev/null; then
        held=$((held + 1))
    fi
done
queue=0
if [[ -d $slot_dir/queue ]]; then
    queue=$(find "$slot_dir/queue" -maxdepth 1 -type f -print 2>/dev/null | wc -l)
fi
printf 'slots: %s/%s held, %s queued\n' "$held" "$slots" "$queue"

available=$(df -B1 --output=avail / 2>/dev/null | tail -1)
case $available in
    ''|*[!0-9]*)
        printf 'disk: cannot read free space\n'
        ;;
    *)
        printf 'disk: %s GiB free (floor %s GiB)\n' \
            "$((available / 1024 / 1024 / 1024))" "$min_gb"
        ;;
esac

printf 'load: %s; cpus: %s\n' \
    "$(awk '{print $1, $2, $3}' /proc/loadavg 2>/dev/null || printf unavailable)" \
    "$(nproc 2>/dev/null || printf unavailable)"
free -h 2>/dev/null | sed 's/^/ram: /' || printf 'ram: unavailable\n'

if command -v sccache >/dev/null 2>&1; then
    sccache --show-stats 2>&1 | sed 's/^/sccache: /'
else
    printf 'sccache: not on PATH\n'
fi

orphans=$(ps -eo pid,ppid,cmd 2>/dev/null |
    awk '$2 == 1 && /cargo|rustc|sccache/ {print}')
if [[ -n $orphans ]]; then
    printf 'orphans:\n%s\n' "$orphans" | sed 's/^/  /'
else
    printf 'orphans: none\n'
fi
REMOTE
    then
        printf '%s: unreachable (ssh failed)\n' "$host"
    fi
done

printf '\n===== local doctor =====\n'
if ! bash "$repository/scripts/doctor.sh" --quiet; then
    printf 'doctor: reported problems above (or could not run)\n'
fi
