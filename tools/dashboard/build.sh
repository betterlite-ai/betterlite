#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tools="${root}/tools/dashboard"
work="${root}/target/dashboard"
bundle_tool="${tools}/bundle.mjs"

fail() {
  printf 'build.sh: %s\n' "$1" >&2
  exit 1
}

usage() {
  fail 'usage: build.sh key|build|check <tests-directory> [--api-base <url>] [--rebuild] [--source-repo <checkout>] [--out <directory>]'
}

[[ $# -gt 0 ]] || usage
command="$1"
shift
tests_dir=""
if [[ ${command} == check ]]; then
  [[ $# -gt 0 ]] || usage
  tests_dir="$1"
  shift
fi

api_base=""
rebuild=false
source_args=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --rebuild)
      rebuild=true
      shift
      ;;
    --api-base | --source-repo | --out | --ui-base-path)
      [[ $# -ge 2 ]] || fail "$1 needs a value"
      case "$1" in
        --api-base) api_base="$2" ;;
        --source-repo) source_args=(--source-repo "$2") ;;
        --out) work="$2" ;;
        *) printf 'build.sh: --ui-base-path is ignored, next.config.mjs never reads it\n' >&2 ;;
      esac
      shift 2
      ;;
    *) usage ;;
  esac
done

# The export is the same-origin production build unless --api-base says otherwise
unset NEXT_PUBLIC_BASE_URL NEXT_PUBLIC_USE_REWRITES
if [[ -n ${UI_BASE_PATH:-} ]]; then
  printf 'build.sh: UI_BASE_PATH is ignored, next.config.mjs never reads it\n' >&2
fi
dist="${work}/dist"
bundle_args=(--api-base "${api_base}")
key="$(node "${bundle_tool}" key "${bundle_args[@]}")"
[[ ${command} != key ]] || {
  printf '%s\n' "${key}"
  exit 0
}

cache_line="$(node "${bundle_tool}" cache-flags)"
read -r -a cache_flags <<< "${cache_line}"

prepare_source() {
  node "${tools}/source.mjs" prepare --out "${work}/source" --cache "${work}/cache" "${source_args[@]}"
}

buildx() {
  # the gate's task container carries Node and the host's docker socket but no docker client
  if ! command -v docker > /dev/null; then
    command -v apt-get > /dev/null || fail 'docker is not installed and this host has no apt-get'
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends docker-cli docker-buildx
  fi
  docker buildx build --file "${root}/packaging/ui/Dockerfile" --build-context "tools=${tools}" "${cache_flags[@]}" "$@"
}

build() {
  if [[ ${rebuild} == false && -f ${dist}/ui/.litellm_ui_ready && -f ${dist}/manifest.json ]] && grep -Fq "\"key\": \"${key}\"" "${dist}/manifest.json"; then
    printf 'dashboard bundle %s is current\n' "${key}"
    return
  fi
  local build_id inputs
  local cache_args=()
  if [[ ${rebuild} == true ]]; then
    cache_args=(--no-cache-filter compiled --no-cache-filter bundled)
  fi
  build_id="$(node "${bundle_tool}" build-id "${bundle_args[@]}")"
  inputs="$(node "${bundle_tool}" inputs "${bundle_args[@]}")"
  prepare_source
  rm -rf "${work}/export"
  buildx --target export "${cache_args[@]}" --build-arg "BUILD_ID=${build_id}" --build-arg "API_BASE=${api_base}" \
    --build-arg "MANIFEST_INPUTS=${inputs}" --output "type=local,dest=${work}/export" "${work}/source"
  rm -rf "${dist}"
  mv "${work}/export/bundle" "${dist}"
  rmdir "${work}/export"
  printf 'dashboard bundle %s written to %s\n' "${key}" "${dist}"
}

check() {
  prepare_source
  node "${bundle_tool}" plan --tests "${tests_dir}" --app "${work}/source/ui/litellm-dashboard" --out "${work}/check"
  rm -rf "${work}/checked"
  buildx --target checked --build-context "checkplan=${work}/check" --output "type=local,dest=${work}/checked" "${work}/source"
  printf 'dashboard checks passed for %s\n' "${tests_dir}"
}

case "${command}" in
  build) build ;;
  check) check ;;
  *) usage ;;
esac
