#!/usr/bin/env bash
# The dispatcher lints a done answer here before review: clippy failed 886 PR runs against 43 queue runs by 11-19, and a lane run takes 2 to 6 minutes where a CI round takes 39 (lever 2, Max 11-18). The clippy check calls it with its package flags when GATE_LINT_ROOT is set.
set -u
root=${GATE_LINT_ROOT:?next-slot.sh sets it}
lanes=${GATE_LINT_LANES:-2}
mkdir -p "${root}" target/lint-lane || exit 1
branch=$(git branch --show-current) || exit 1
id=${branch#task/}
id=${id%%-*}
packages=$(($# / 2))
started=${EPOCHSECONDS}
lane=0
# each lane has its own lean target, so two runs never wait on one cargo lock
while [[ "${lane}" -eq 0 ]]; do
  for ((number = 1; number <= lanes; number++)); do
    exec 9> "${root}/lane-${number}.lock"
    if flock -n 9; then
      lane=${number}
      break
    fi
  done
  if [[ "${lane}" -eq 0 ]]; then
    if [[ $((EPOCHSECONDS - started)) -ge 600 ]]; then
      printf '%(%FT%T)T,%s,0,600,0,busy,%s,0\n' -1 "${id}" "${packages}" >> "${root}/runs.csv"
      echo "every lint lane stayed busy for 10 minutes, so CI lints this change"
      exit 0
    fi
    sleep 5
  fi
done
waited=$((EPOCHSECONDS - started))
target=${root}/target-${lane}
size=$(du -s -BG "${target}" 2> /dev/null) || size=0
# a lean target grew to 5.6 GB over a night of probes; past 20 GB the next run starts cold
if [[ "${size%%G*}" -gt 20 ]]; then
  rm -rf -- "${target}"
fi
export CARGO_TARGET_DIR=${target} RUSTC_WRAPPER='' CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=7
began=${EPOCHSECONDS}
timeout 1800 cargo clippy --keep-going --all-targets --locked --message-format=short "$@" -- -D warnings > target/lint-lane/clippy.txt 2>&1
code=$?
size=$(du -s -BG "${target}" 2> /dev/null) || size=0
printf '%(%FT%T)T,%s,%s,%s,%s,%s,%s,%s\n' -1 "${id}" "${lane}" "${waited}" "$((EPOCHSECONDS - began))" "${code}" "${packages}" "${size%%G*}" >> "${root}/runs.csv"
case "${code}" in
  0)
    echo "lint lane ${lane}: clippy passed"
    ;;
  124)
    echo "the lint lane ran past 30 minutes, so CI lints this change"
    code=0
    ;;
  *)
    echo "clippy failed in the lint lane; every error is in target/lint-lane/clippy.txt in your worktree"
    grep -E ': (error|warning)' target/lint-lane/clippy.txt
    ;;
esac
exit "${code}"
