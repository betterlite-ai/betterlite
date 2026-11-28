#!/usr/bin/env bash
# The dispatcher lints a done answer here before review: clippy failed 886 PR runs against 43 queue runs by 11-19, and a lane run takes 2 to 6 minutes where a CI round takes 39 (lever 2, Max 11-18). The clippy check calls it with its package flags when GATE_LINT_ROOT is set.
set -u
root=${GATE_LINT_ROOT:?next-slot.sh sets it}
lanes=${GATE_LINT_LANES:-2}
mkdir -p "${root}" target/lint-lane || exit 1
[[ -e "${root}/runs.csv" ]] || echo "time,task,lane,wait_s,run_s,rc,packages,target_gb" > "${root}/runs.csv"
branch=$(git branch --show-current) || exit 1
id=${branch#task/}
id=${id%%-*}
packages=$(($# / 2))
started=${EPOCHSECONDS}
# a task within 10 steps of the longest open chain (build slot 1 in next-slot.sh) marks that it waits, and every other task leaves the next free lane to it (lever 1 of the 11-20 plan, Max 11-28)
chains=${root}/chain-waits
mkdir -p "${chains}" || exit 1
mark=""
if [[ "${GATE_LOWEST_BUILD_SLOT:-2}" -eq 1 ]]; then
  mark=${chains}/${id}
  echo "$$" > "${mark}" || exit 1
  trap 'rm -f -- "${mark}"' EXIT
fi
lane=0
# each lane has its own lean target, so two runs never wait on one cargo lock
while [[ "${lane}" -eq 0 ]]; do
  yield=0
  if [[ -z "${mark}" ]]; then
    for file in "${chains}"/*; do
      # a mark whose process is gone, or is no lint lane any more, holds nobody back
      [[ -e "${file}" ]] && grep -q -a -F lint-lane.sh "/proc/$(< "${file}")/cmdline" 2> /dev/null && yield=1
    done
  fi
  for ((number = 1; yield == 0 && number <= lanes; number++)); do
    exec 9> "${root}/lane-${number}.lock"
    if flock -n 9; then
      lane=${number}
      break
    fi
  done
  if [[ "${lane}" -eq 0 ]]; then
    if [[ $((EPOCHSECONDS - started)) -ge 1200 ]]; then
      printf '%(%FT%T)T,%s,0,1200,0,busy,%s,0\n' -1 "${id}" "${packages}" >> "${root}/runs.csv"
      echo "every lint lane stayed busy for 20 minutes, so CI lints this change"
      exit 0
    fi
    sleep 5
  fi
done
[[ -n "${mark}" ]] && rm -f -- "${mark}"
waited=$((EPOCHSECONDS - started))
target=${root}/target-${lane}
size=$(du -s -BG "${target}" 2> /dev/null) || size=0
# a lean target grew to 5.6 GB over a night of probes; past 20 GB the next run starts cold
if [[ "${size%%G*}" -gt 20 ]]; then
  rm -rf -- "${target}"
fi
export CARGO_TARGET_DIR=${target} RUSTC_WRAPPER='' CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=7
# cargo hashes a workspace crate by its path inside the workspace, so all worktrees share one build of it and file mtimes alone decide whether it is fresh
trees=${target}/trees.tsv
lane_rows=$(awk -F, -v lane="${lane}" '$3 == lane' "${root}/runs.csv")
recorded=""
[[ -e "${target}/last-row" ]] && recorded=$(< "${target}/last-row")
# a run by an older lint-lane.sh changed builds without updating the record
if [[ "${lane_rows##*$'\n'}" != "${recorded}" ]]; then
  rm -f -- "${trees}"
fi
if ! metadata=$(cargo metadata --no-deps --format-version 1 --locked) || ! members=$(jq -r '.packages[] | "\(.name)\t\(.manifest_path)"' <<< "${metadata}"); then
  echo "the lint lane could not list the workspace crates, so CI lints this change"
  exit 0
fi
built=""
while IFS=$'\t' read -r name manifest; do
  dir=${manifest%/Cargo.toml}
  dir=${dir#"${PWD}"}
  if ! tree=$(git rev-parse "HEAD:${dir#/}"); then
    echo "the lint lane could not read the tree of ${name}, so CI lints this change"
    exit 0
  fi
  if ! grep -q -x -F "${name}"$'\t'"${tree}" "${trees}" 2> /dev/null; then
    find "${target}/debug/.fingerprint" -maxdepth 1 -regextype posix-extended -regex ".*/${name}-[0-9a-f]{16}" -exec rm -rf -- {} + 2> /dev/null
  fi
  built+="${name}"$'\t'"${tree}"$'\n'
done <<< "${members}"
printf '%s' "${built}" > "${trees}"
began=${EPOCHSECONDS}
timeout 1800 cargo clippy --keep-going --all-targets --locked --message-format=short "$@" -- -D warnings > target/lint-lane/clippy.txt 2>&1
code=$?
size=$(du -s -BG "${target}" 2> /dev/null) || size=0
row=$(printf '%(%FT%T)T,%s,%s,%s,%s,%s,%s,%s' -1 "${id}" "${lane}" "${waited}" "$((EPOCHSECONDS - began))" "${code}" "${packages}" "${size%%G*}")
echo "${row}" >> "${root}/runs.csv"
echo "${row}" > "${target}/last-row"
case "${code}" in
  0)
    echo "lint lane ${lane}: clippy passed"
    ;;
  124)
    echo "the lint lane ran past 30 minutes, so CI lints this change"
    code=0
    ;;
  *)
    # an OOM kill or a cargo failure leaves no compiler error line, and the worker would get a fix turn with nothing to fix
    if ! grep -q -E ': (error|warning)' target/lint-lane/clippy.txt; then
      echo "the lint lane failed without a clippy error (exit ${code}), so CI lints this change"
      exit 0
    fi
    echo "clippy failed in the lint lane; every error is in target/lint-lane/clippy.txt in your worktree"
    grep -E ': (error|warning)' target/lint-lane/clippy.txt
    ;;
esac
exit "${code}"
