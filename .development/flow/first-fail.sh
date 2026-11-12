#!/usr/bin/env bash
# Prints the first two distinct FAIL lines of a gate run, each followed by a space; a log that gh can't fetch or that has none gives nothing. Usage: bash first-fail.sh <run id>, with GH_REPO and GH_TOKEN set
log=$(gh run view "$1" --log-failed 2> /dev/null)
fails=$(grep -a -o -E 'FAIL [a-z-]+ \([0-9]+s\): .*|FAIL \[ *[0-9.]+s\] (\([0-9]+/[0-9]+\) )?[A-Za-z0-9_-]+ [^ ]+' <<< "${log}") || exit 0
awk -v ORS=' ' '{ sub(/\([0-9]+\/[0-9]+\) /, "") } !seen[$0]++ { print; if (++n == 2) exit }' <<< "${fails}"
