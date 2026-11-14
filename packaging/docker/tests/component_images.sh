#!/bin/bash
set -u

base=.
[[ ${BASH_SOURCE[0]} == */* ]] && base=${BASH_SOURCE[0]%/*}
here="$(cd -- "${base}/.." && pwd)"
gateway="${here}/Dockerfile.gateway"
backend="${here}/Dockerfile.backend"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

expect_header() {
  local file=$1
  shift
  local -a expected=("$@")
  local -a actual=()
  mapfile -t actual < "${file}"
  local index
  for index in "${!expected[@]}"; do
    [[ ${actual[index]-} == "${expected[index]}" ]] || fail "${file}: line $((index + 1)) should be [${expected[index]}] but is [${actual[index]-}]"
  done
}

expect_line() {
  local file=$1 line=$2 problem=$3
  grep -Fxq -- "${line}" "${file}" || fail "${problem}: ${file} has no line [${line}]"
}

expect_text() {
  local file=$1 text=$2 problem=$3
  grep -Fq -- "${text}" "${file}" || fail "${problem}: ${file} does not contain [${text}]"
}

refute_text() {
  local file=$1 text=$2 problem=$3
  if grep -Fqi -- "${text}" "${file}"; then
    fail "${problem}: ${file} contains [${text}]"
  fi
}

[[ -f "${gateway}" ]] || fail "missing file ${gateway}"
[[ -f "${backend}" ]] || fail "missing file ${backend}"

wolfi="cgr.dev/chainguard/wolfi-base@sha256:1d95114038f76513a9ace6fca107d5582b08c65981f81f61cb56bf7fd2ef216d"
expect_header "${gateway}" \
  "ARG LITELLM_BUILD_IMAGE=${wolfi}" \
  "ARG LITELLM_RUNTIME_IMAGE=${wolfi}" \
  "ARG PGBOUNCER_VERSION=1.26.0" \
  "ARG PGBOUNCER_SHA256=afd25dd61ee6775d37b40629b87ce08736b3e6955f3057bb212e410fbf21c71d"
expect_header "${backend}" "ARG LITELLM_RUNTIME_IMAGE=${wolfi}"

expect_text "${gateway}" "cargo build --release --locked -p betterlite-gateway" "gateway build command"
refute_text "${gateway}" "-p betterlite-backend" "gateway builds the backend binary"
expect_text "${backend}" "cargo build --release --locked -p betterlite-backend" "backend build command"
refute_text "${backend}" "-p betterlite-gateway" "backend builds the gateway binary"

entrypoint_line="RUN sed -i 's/\\r\$//' /app/docker/component_entrypoint.sh && chmod +x /app/docker/component_entrypoint.sh"
expect_line "${gateway}" "${entrypoint_line}" "gateway entrypoint script setup"
expect_line "${backend}" "${entrypoint_line}" "backend entrypoint script setup"

expect_line "${gateway}" "USER nonroot" "gateway runs as nonroot"
expect_line "${backend}" "USER nonroot" "backend runs as nonroot"

expect_line "${gateway}" "EXPOSE 4000/tcp" "gateway exposes port 4000"
expect_text "${gateway}" "FROM \$LITELLM_BUILD_IMAGE AS pgbouncer-builder" "gateway pgbouncer stage"
expect_text "${gateway}" "COPY --from=pgbouncer-builder /usr/local/bin/pgbouncer /usr/local/bin/pgbouncer" "gateway pgbouncer binary"
expect_text "${gateway}" "apk add --no-cache bash openssl tzdata libevent" "gateway runtime packages"
expect_text "${backend}" "apk add --no-cache bash openssl tzdata && break;" "backend runtime packages"
expect_line "${backend}" "EXPOSE 4001/tcp" "backend exposes port 4001"
refute_text "${backend}" "pgbouncer" "backend pgbouncer"
refute_text "${backend}" "libevent" "backend pgbouncer dependency"

expect_line "${gateway}" 'ENTRYPOINT ["/app/docker/component_entrypoint.sh", "betterlite-gateway"]' "gateway entrypoint"
expect_line "${gateway}" 'CMD ["--host", "0.0.0.0", "--port", "4000"]' "gateway command"
expect_line "${backend}" 'ENTRYPOINT ["/app/docker/component_entrypoint.sh", "betterlite-backend"]' "backend entrypoint"
expect_line "${backend}" 'CMD ["--host", "0.0.0.0", "--port", "4001"]' "backend command"

for file in "${gateway}" "${backend}"; do
  for text in python "uv sync" prisma node PYTHONPATH; do
    refute_text "${file}" "${text}" "Python stack"
  done
done

echo "ok"
