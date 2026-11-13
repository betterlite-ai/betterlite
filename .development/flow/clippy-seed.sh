#!/usr/bin/env bash
# Keeps a clippy target of main in betterlite-worktrees/clippy-seed for the gate to copy into each new task checkout (dispatch/worktree.rs). watchdog.sh starts it every 20 minutes. It lints betterlite-oracle with --lib and with --all-targets, the first clippy runs of most workers and of the gate (11-12: 165 of 746 Bash calls killed at 10 minutes were `cargo clippy -p betterlite-oracle --lib`, and a seeded first run took 319 s against 979 s cold). The snapshot only changes under an exclusive lock, so a gate copy never sees a half-written target.
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
L=${worktrees}/logs
store=${worktrees}/clippy-seed
checkout=${store}/checkout
if [[ -e "${main}/.git/betterlite/maintenance" ]] || [[ -e "${L}/maintenance.STOP" ]]; then exit 0; fi
mkdir -p "${store}" || exit 1
exec 8> "${store}/refresh" || exit 1
flock -n 8 || exit 0
if [[ ! -d "${checkout}/.git" ]]; then
  git clone --quiet --local --no-checkout "${main}" "${checkout}" || exit 1
  coordinator=$(git -C "${main}" rev-parse --path-format=absolute --git-common-dir) || exit 1
  git -C "${checkout}" config betterlite.coordinator "${coordinator}" || exit 1
fi
git -C "${checkout}" fetch --quiet origin || exit 1
git -C "${checkout}" checkout --quiet --detach origin/main || exit 1
commit=$(git -C "${checkout}" rev-parse HEAD) || exit 1
if [[ -f "${store}/commit" ]] && [[ $(< "${store}/commit") == "${commit}" ]]; then exit 0; fi
cd "${checkout}" || exit 1
printf '%(%H:%M)T building %s\n' -1 "${commit}"
for targets in --lib --all-targets; do
  PATH=${P}/bin:${PATH} GATE_BUILD_SLOTS=8 GATE_LOWEST_BUILD_SLOT=2 systemd-run --user --scope --slice=betterlite-dispatch.slice -p MemoryMax=16G -p MemorySwapMax=4G -q -- bash "${P}/build-env.sh" nice -n 10 cargo clippy --quiet --locked -p betterlite-oracle "${targets}" -- -D warnings 8>&- || exit 1
done
rm -rf "${store}/target.new" || exit 1
cp -a target "${store}/target.new" || exit 1
exec 9> "${store}/lock" || exit 1
flock 9 || exit 1
rm -rf "${store}/target.old" || exit 1
if [[ -d "${store}/target" ]]; then mv "${store}/target" "${store}/target.old" || exit 1; fi
mv "${store}/target.new" "${store}/target" || exit 1
printf '%s\n' "${commit}" > "${store}/commit"
flock -u 9
rm -rf "${store}/target.old"
printf '%(%H:%M)T published %s\n' -1 "${commit}"
