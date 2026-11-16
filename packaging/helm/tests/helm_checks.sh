#!/bin/sh
set -eu

if [ "$#" -lt 2 ]; then
  echo "usage: $0 lint <chart-dir> [values-file]" >&2
  echo "       $0 unittest <chart-dir> <suite-file>..." >&2
  exit 2
fi

mode=$1
chart=$2
shift 2

if [ ! -d "${chart}" ]; then
  echo "no chart directory ${chart}" >&2
  exit 1
fi

copy=$(mktemp -d)
output=$(mktemp)
trap 'rm -rf "${copy}" "${output}"' EXIT

cp -R "${chart}/." "${copy}"

# helm resolves the subcharts of every render, even the disabled ones, so both modes build them in the copy
if grep -q '^dependencies:' "${copy}/Chart.yaml"; then
  helm dependency build "${copy}"
fi

case "${mode}" in
  lint)
    if [ "$#" -gt 1 ]; then
      echo "lint takes at most one values file" >&2
      exit 2
    fi
    if [ "$#" -eq 1 ]; then
      helm lint --strict -f "$1" "${copy}"
      helm template release "${copy}" -f "$1" > /dev/null
    else
      helm lint --strict "${copy}"
      helm template release "${copy}" > /dev/null
    fi
    ;;
  unittest)
    if [ "$#" -eq 0 ]; then
      echo "unittest needs at least one suite file" >&2
      exit 2
    fi
    declared=0
    for suite in "$@"; do
      if [ ! -f "${chart}/${suite}" ]; then
        echo "no suite file ${chart}/${suite}" >&2
        exit 1
      fi
      count=$(awk '/^suite:/ {count++} END {print count + 0}' "${chart}/${suite}")
      declared=$((declared + count))
    done
    # rebuild the arguments as one -f per suite without word splitting
    suites=$#
    index=0
    while [ "${index}" -lt "${suites}" ]; do
      suite=$1
      shift
      set -- "$@" -f "${suite}"
      index=$((index + 1))
    done
    if ! helm unittest --color=false "$@" "${copy}" > "${output}" 2>&1; then
      cat "${output}"
      exit 1
    fi
    cat "${output}"
    executed=$(sed -n 's/^Test Suites:.*[[:space:]]\([0-9][0-9]*\) total$/\1/p' "${output}")
    if [ "${declared}" != "${executed}" ]; then
      echo "declared ${declared} suites, helm unittest ran ${executed:-none}" >&2
      exit 1
    fi
    ;;
  *)
    echo "unknown mode ${mode}" >&2
    exit 2
    ;;
esac
