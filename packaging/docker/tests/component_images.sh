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
  local -a wanted=("$@")
  local -a actual=()
  mapfile -t actual < "${file}"
  local index
  for index in "${!wanted[@]}"; do
    [[ ${actual[index]-} == "${wanted[index]}" ]] || fail "${file}: line $((index + 1)) should be [${wanted[index]}] but is [${actual[index]-}]"
  done
}

expect_line() {
  local file=$1 line=$2 problem=$3
  grep -Fxq -- "${line}" "${file}" || fail "${problem}: ${file} has no line [${line}]"
}

expect_last_line() {
  local file=$1 keyword=$2 expected=$3 problem=$4
  local -a lines=()
  mapfile -t lines < "${file}"
  local line last=
  for line in "${lines[@]}"; do
    if [[ ${line} == "${keyword}"* ]]; then
      last=${line}
    fi
  done
  [[ ${last} == "${expected}" ]] || fail "${problem}: the last ${keyword} line in ${file} is [${last}], expected [${expected}]"
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

expect_line "${gateway}" "FROM rust:1.98.0-bookworm AS builder" "gateway builder stage"
expect_line "${backend}" "FROM rust:1.98.0-bookworm AS builder" "backend builder stage"
expect_line "${gateway}" "COPY Cargo.toml Cargo.lock rust-toolchain.toml /app/" "gateway builder manifest copy"
expect_line "${backend}" "COPY Cargo.toml Cargo.lock rust-toolchain.toml /app/" "backend builder manifest copy"
expect_line "${gateway}" "COPY crates/ /app/crates/" "gateway builder crate copy"
expect_line "${backend}" "COPY crates/ /app/crates/" "backend builder crate copy"
expect_line "${gateway}" "COPY tools/ /app/tools/" "gateway builder tool copy"
expect_line "${backend}" "COPY tools/ /app/tools/" "backend builder tool copy"
expect_line "${gateway}" "COPY packaging/docker/component_entrypoint.sh /app/docker/component_entrypoint.sh" "gateway entrypoint source copy"
expect_line "${backend}" "COPY packaging/docker/component_entrypoint.sh /app/docker/component_entrypoint.sh" "backend entrypoint source copy"

expect_text "${gateway}" "cargo build --release --locked -p betterlite-gateway" "gateway build command"
refute_text "${gateway}" "-p betterlite-backend" "gateway builds the backend binary"
expect_text "${backend}" "cargo build --release --locked -p betterlite-backend" "backend build command"
refute_text "${backend}" "-p betterlite-gateway" "backend builds the gateway binary"

entrypoint_line="RUN sed -i 's/\\r\$//' /app/docker/component_entrypoint.sh && chmod +x /app/docker/component_entrypoint.sh"
expect_line "${gateway}" "${entrypoint_line}" "gateway entrypoint script setup"
expect_line "${backend}" "${entrypoint_line}" "backend entrypoint script setup"

expect_last_line "${gateway}" "FROM" "FROM \$LITELLM_RUNTIME_IMAGE AS runtime" "gateway runtime stage"
expect_last_line "${backend}" "FROM" "FROM \$LITELLM_RUNTIME_IMAGE AS runtime" "backend runtime stage"
expect_line "${gateway}" "USER root" "gateway installs packages as root"
expect_line "${backend}" "USER root" "backend installs packages as root"
expect_text "${gateway}" "for i in 1 2 3; do" "gateway apk retry loop"
expect_text "${backend}" "for i in 1 2 3; do" "backend apk retry loop"
expect_text "${gateway}" "apk add failed after 3 retries" "gateway apk retry failure"
expect_text "${backend}" "apk add failed after 3 retries" "backend apk retry failure"
expect_last_line "${gateway}" "WORKDIR" "WORKDIR /app" "gateway workdir"
expect_last_line "${backend}" "WORKDIR" "WORKDIR /app" "backend workdir"
expect_last_line "${gateway}" "ENV HOME" "ENV HOME=/home/nonroot" "gateway home directory"
expect_last_line "${backend}" "ENV HOME" "ENV HOME=/home/nonroot" "backend home directory"
expect_line "${gateway}" "COPY --from=builder /app/target/release/betterlite-gateway /usr/local/bin/betterlite-gateway" "gateway binary copy"
expect_line "${backend}" "COPY --from=builder /app/target/release/betterlite-backend /usr/local/bin/betterlite-backend" "backend binary copy"
expect_line "${gateway}" "COPY --from=builder --chown=nonroot:nonroot /app/docker /app/docker" "gateway entrypoint copy"
expect_line "${backend}" "COPY --from=builder --chown=nonroot:nonroot /app/docker /app/docker" "backend entrypoint copy"

expect_last_line "${gateway}" "USER" "USER nonroot" "gateway runs as nonroot"
expect_last_line "${backend}" "USER" "USER nonroot" "backend runs as nonroot"

expect_last_line "${gateway}" "EXPOSE" "EXPOSE 4000/tcp" "gateway exposes port 4000"
expect_text "${gateway}" "FROM \$LITELLM_BUILD_IMAGE AS pgbouncer-builder" "gateway pgbouncer stage"
expect_line "${gateway}" "ARG PGBOUNCER_VERSION" "gateway pgbouncer version arg"
expect_line "${gateway}" "ARG PGBOUNCER_SHA256" "gateway pgbouncer checksum arg"
expect_text "${gateway}" "curl -fsSL -o pgbouncer.tar.gz \"https://www.pgbouncer.org/downloads/files/\${PGBOUNCER_VERSION}/pgbouncer-\${PGBOUNCER_VERSION}.tar.gz\"" "gateway pgbouncer download"
expect_text "${gateway}" "echo \"\${PGBOUNCER_SHA256}  pgbouncer.tar.gz\" | sha256sum -c -" "gateway pgbouncer checksum verification"
expect_text "${gateway}" "./configure --prefix=/usr/local --with-openssl=/usr" "gateway pgbouncer configure"
expect_text "${gateway}" "make -j\"\$(nproc)\" pgbouncer" "gateway pgbouncer build"
expect_text "${gateway}" "install -m 0755 pgbouncer /usr/local/bin/pgbouncer" "gateway pgbouncer install"
expect_text "${gateway}" "COPY --from=pgbouncer-builder /usr/local/bin/pgbouncer /usr/local/bin/pgbouncer" "gateway pgbouncer binary"
expect_text "${gateway}" "apk add --no-cache bash openssl tzdata libevent" "gateway runtime packages"
expect_text "${backend}" "apk add --no-cache bash openssl tzdata && break;" "backend runtime packages"
expect_last_line "${backend}" "EXPOSE" "EXPOSE 4001/tcp" "backend exposes port 4001"
refute_text "${backend}" "pgbouncer" "backend pgbouncer"
refute_text "${backend}" "libevent" "backend pgbouncer dependency"

expect_last_line "${gateway}" "ENTRYPOINT" 'ENTRYPOINT ["/app/docker/component_entrypoint.sh", "betterlite-gateway"]' "gateway entrypoint"
expect_last_line "${gateway}" "CMD" 'CMD ["--host", "0.0.0.0", "--port", "4000"]' "gateway command"
expect_last_line "${backend}" "ENTRYPOINT" 'ENTRYPOINT ["/app/docker/component_entrypoint.sh", "betterlite-backend"]' "backend entrypoint"
expect_last_line "${backend}" "CMD" 'CMD ["--host", "0.0.0.0", "--port", "4001"]' "backend command"

for file in "${gateway}" "${backend}"; do
  for text in python "uv sync" prisma node PYTHONPATH; do
    refute_text "${file}" "${text}" "Python stack"
  done
done

echo "ok"
