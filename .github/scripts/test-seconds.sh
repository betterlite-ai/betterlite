#!/usr/bin/env bash
# Writes .config/test-seconds.toml from the tests parts of one green main gate run: bash .github/scripts/test-seconds.sh <run id>
set -euo pipefail
run=$1
repository=betterlite-ai/betterlite
jobs=$(gh api "repos/${repository}/actions/runs/${run}/jobs?per_page=100" --jq '.jobs[] | select(.name | startswith("tests-")) | .id')
[[ -n ${jobs} ]] || {
  echo "run ${run} has no tests parts" >&2
  exit 1
}
# nextest prints "PASS [   0.009s] (    1/22675) <binary id> <test>", and its final summary repeats failures without the counter
for job in ${jobs}; do
  gh api --allow-escape-sequences "repos/${repository}/actions/jobs/${job}/logs"
done | LC_ALL=C awk '
  / (PASS|FAIL|FLAKY|LEAK|TIMEOUT) \[ *[0-9.]+s\] \( *[0-9]+\/[0-9]+\) / {
    line = $0
    sub(/.* (PASS|FAIL|FLAKY|LEAK|TIMEOUT) \[ */, "", line)
    seconds = line + 0
    sub(/^[0-9.]+s\] \( *[0-9]+\/[0-9]+\) /, "", line)
    split(line, fields, " ")
    package = fields[1]
    sub(/::.*/, "", package)
    total[package] += seconds
  }
  END { for (package in total) printf "%s = %d\n", package, total[package] + 0.5 }
' | LC_ALL=C sort > .config/test-seconds.toml
