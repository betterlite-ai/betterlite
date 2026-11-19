#!/bin/sh
set -eu

for chart in packaging/helm/betterlite-helm packaging/helm/betterlite; do
  files=0
  set --
  for suite in "${chart}"/tests/*_tests.yaml; do
    if [ ! -f "${suite}" ]; then
      continue
    fi
    files=$((files + 1))
    set -- "$@" "tests/${suite##*/}"
  done
  if [ "${files}" -eq 0 ]; then
    echo "no suite files in ${chart}" >&2
    exit 1
  fi
  if ! output=$(sh packaging/helm/tests/helm_checks.sh unittest "${chart}" "$@"); then
    printf '%s\n' "${output}"
    exit 1
  fi
  # helm_checks.sh prints helm unittest's own summary, which check_suites.sh reads back
  ran=$(printf '%s\n' "${output}" | sed -n 's/^Test Suites:.*[[:space:]]\([0-9][0-9]*\) total$/\1/p')
  if [ "${ran}" != "${files}" ]; then
    echo "suite count mismatch in ${chart}: ${ran} ran, ${files} files" >&2
    exit 1
  fi
  echo "ok ${chart} ${ran} suites"
done
