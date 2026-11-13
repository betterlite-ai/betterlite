#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${root}"

fail() {
  printf 'files-check: %s\n' "$1" >&2
  exit 1
}

contains() {
  grep -Fq -- "$2" "$1" || fail "$1 is missing: $2"
}

for file in Dockerfile Dockerfile.dockerignore compose.yaml compose.build.yaml; do
  test -f "packaging/lens/${file}" || fail "packaging/lens/${file} does not exist"
done

contains packaging/lens/Dockerfile 'FROM rust:1.98.0-bookworm AS builder'
contains packaging/lens/Dockerfile 'cargo build --release --locked -p betterlite-lens-worker'
contains packaging/lens/Dockerfile 'install -m 0755 target/release/betterlite-lens-worker /usr/local/bin/betterlite-lens-worker'
contains packaging/lens/Dockerfile 'FROM cgr.dev/chainguard/wolfi-base@sha256:1d95114038f76513a9ace6fca107d5582b08c65981f81f61cb56bf7fd2ef216d'
contains packaging/lens/Dockerfile 'COPY --from=builder /usr/local/bin/betterlite-lens-worker /usr/local/bin/betterlite-lens-worker'
contains packaging/lens/Dockerfile 'USER 65532:65532'
contains packaging/lens/Dockerfile 'ENTRYPOINT ["/usr/local/bin/betterlite-lens-worker"]'
if grep -Eqi 'python|prisma|apk add|apt-get|pip install|npm install' packaging/lens/Dockerfile; then
  fail 'the image installs Python, Prisma or shell tooling'
fi

contains packaging/lens/Dockerfile.dockerignore '**'
contains packaging/lens/Dockerfile.dockerignore '!Cargo.toml'
contains packaging/lens/Dockerfile.dockerignore '!Cargo.lock'
contains packaging/lens/Dockerfile.dockerignore '!rust-toolchain.toml'
contains packaging/lens/Dockerfile.dockerignore '!crates/'
contains packaging/lens/Dockerfile.dockerignore '!tools/'
contains packaging/lens/Dockerfile.dockerignore '**/target'

contains packaging/lens/compose.yaml 'lens-worker:'
contains packaging/lens/compose.yaml "image: \${LENS_WORKER_IMAGE:?Set the worker image, build it with compose.build.yaml}"
contains packaging/lens/compose.yaml "LITELLM_URL: \${LITELLM_URL:?Set the URL reachable from this container}"
contains packaging/lens/compose.yaml "LENS_WORKER_TOKEN: \${LENS_WORKER_TOKEN:?Create a worker credential in the Lens UI}"
contains packaging/lens/compose.yaml 'restart: unless-stopped'
contains packaging/lens/compose.yaml 'read_only: true'
contains packaging/lens/compose.yaml "/tmp:rw,noexec,nosuid,size=\${LENS_WORKER_TMP_SIZE:-1g}"
contains packaging/lens/compose.yaml 'cap_drop: [ALL]'
contains packaging/lens/compose.yaml 'security_opt: [no-new-privileges:true]'

contains packaging/lens/compose.build.yaml 'lens-worker:'
contains packaging/lens/compose.build.yaml 'context: ../..'
contains packaging/lens/compose.build.yaml 'dockerfile: packaging/lens/Dockerfile'
contains packaging/lens/compose.build.yaml 'image: betterlite-lens-worker:local'
