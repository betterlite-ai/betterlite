#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tools="${root}/tools/dashboard"
dist="${root}/target/dashboard/dist"
ui="${dist}/ui"
scratch="$(mktemp -d)"
trap 'rm -rf "${scratch}"' EXIT

fail() {
  printf 'check-build: %s\n' "$1" >&2
  exit 1
}

# the gate's task container carries Node and the host's docker socket but no docker client
if ! command -v docker > /dev/null; then
  command -v apt-get > /dev/null || fail 'docker is not installed and this host has no apt-get'
  apt-get update
  DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends docker-cli docker-buildx
fi

# One line per file: path, mode and content digest
snapshot() {
  local files file mode digest
  files="$(cd "$1" && find . -type f | LC_ALL=C sort)"
  while read -r file; do
    mode="$(stat -c %a "$1/${file}")"
    digest="$(sha256sum "$1/${file}")"
    printf '%s %s %s\n' "${file}" "${mode}" "${digest%% *}"
  done <<< "${files}"
}

archive_digest() {
  local digest
  digest="$(tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner --format=gnu -C "$1" -cf - . | sha256sum)"
  printf '%s\n' "${digest%% *}"
}

# the build sees these, so any that reaches the output is a leak
export LITELLM_MASTER_KEY=sk-sentinel-master-4e1f DATABASE_URL=postgres://sentinel:pw-9c2d@db.invalid/bl PROXY_ADMIN_ID=sentinel-admin-77ab

bash "${tools}/build.sh" build --rebuild "$@" 2>&1 | tee "${scratch}/build.log"
cp -a "${dist}" "${scratch}/first"
bash "${tools}/build.sh" build --rebuild "$@" 2>&1 | tee -a "${scratch}/build.log"

snapshot "${scratch}/first" > "${scratch}/first.txt"
snapshot "${dist}" > "${scratch}/second.txt"
diff -u "${scratch}/first.txt" "${scratch}/second.txt" || fail 'two clean builds differ in path, mode or content'
first_archive="$(archive_digest "${scratch}/first")"
second_archive="$(archive_digest "${dist}")"
[[ ${first_archive} == "${second_archive}" ]] || fail 'two clean builds give different archives'

for required in index.html login/index.html notices/LiteLLM-LICENSE.txt notices/npm-dependencies.txt; do
  [[ -s ${ui}/${required} ]] || fail "the bundle has no ${required}"
done
[[ -f ${ui}/.litellm_ui_ready ]] || fail 'the bundle has no .litellm_ui_ready marker'
cmp "${ui}/notices/LiteLLM-LICENSE.txt" "${root}/target/dashboard/source/LICENSE" || fail 'LiteLLM-LICENSE.txt is not the upstream LICENSE'
scripts="$(find "${ui}/_next/static" -name '*.js' -print -quit)"
payloads="$(find "${ui}" -name 'index.txt' -print -quit)"
segments="$(find "${ui}" -name '__next*.txt' -print -quit)"
[[ -n ${scripts} ]] || fail 'the bundle has no _next/static scripts'
[[ -n ${payloads} ]] || fail 'the bundle has no RSC payloads'
[[ -n ${segments} ]] || fail 'the bundle has no __next segment files'
logos="$(find "${ui}" \( -name logo.png -o -name logo_dark.png -o -name logo_monogram.png -o -name logo_monogram_dark.png -o -name cached_logo.jpg \) -print)"
[[ -z ${logos} ]] || fail "the bundle ships ${logos}"
for field in key buildId builder node npm tree; do
  grep -q "\"${field}\": \"" "${dist}/manifest.json" || fail "manifest.json has no ${field}"
done
for sentinel in sk-sentinel-master-4e1f pw-9c2d sentinel-admin-77ab; do
  if grep -rqF -- "${sentinel}" "${dist}" "${scratch}/build.log"; then
    fail "${sentinel} reached the build output"
  fi
done

key="$(bash "${tools}/build.sh" key)"
with_api_base="$(bash "${tools}/build.sh" key --api-base https://api.example.test)"
[[ ${with_api_base} != "${key}" ]] || fail '--api-base does not change the cache key'
with_base_path="$(UI_BASE_PATH=/ignored bash "${tools}/build.sh" key 2> /dev/null)"
[[ ${with_base_path} == "${key}" ]] || fail 'UI_BASE_PATH changes the cache key'

for part in tools/dashboard packaging/ui ui/dashboard/patches; do
  mkdir -p "${scratch}/repo/${part}"
  cp -a "${root}/${part}/." "${scratch}/repo/${part}/"
done
printf '# changed\n' >> "${scratch}/repo/ui/dashboard/patches/10-build.patch"
with_patch_edit="$(node "${tools}/bundle.mjs" key --repo-root "${scratch}/repo")"
[[ ${with_patch_edit} != "${key}" ]] || fail 'a patch edit does not change the cache key'

# a matching key reuses the export, so Docker and npm must not run
mkdir -p "${scratch}/bin"
printf '#!/bin/sh\necho "must not run" >&2\nexit 99\n' > "${scratch}/bin/docker"
cp "${scratch}/bin/docker" "${scratch}/bin/npm"
chmod +x "${scratch}/bin/docker" "${scratch}/bin/npm"
reuse="$(PATH="${scratch}/bin:${PATH}" bash "${tools}/build.sh" build)"
[[ ${reuse} == *'is current'* ]] || fail 'a matching key did not reuse the export'

# installing replaces the sidecar and drops stale content-hashed files
mkdir -p "${scratch}/prefix/bin/ui/_next/static/stale" "${scratch}/prefix/bin/ui/old-route"
touch "${scratch}/prefix/bin/ui/_next/static/stale/chunk.js" "${scratch}/prefix/bin/ui/old-route/index.html"
printf '#!/bin/sh\n' > "${scratch}/prefix/bin/betterlite"
chmod +x "${scratch}/prefix/bin/betterlite"
node "${tools}/install.mjs" --bundle "${dist}" --bin "${scratch}/prefix/bin/betterlite"
snapshot "${scratch}/prefix/bin/ui" > "${scratch}/installed.txt"
snapshot "${ui}" > "${scratch}/expected.txt"
diff -u "${scratch}/expected.txt" "${scratch}/installed.txt" || fail 'the installed sidecar is not the bundle'

# the shared check entrypoint runs a caller's Vitest files and the upstream eslint and prettier
bash "${tools}/build.sh" check "${tools}/tests/build/web" "$@"
printf 'check-build: ok, bundle %s\n' "${key}"
