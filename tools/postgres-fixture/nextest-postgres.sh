#!/usr/bin/env bash
# nextest setup script: starts one PostgreSQL server for this test run and hands its URL to the tests through NEXTEST_ENV. CI sets POSTGRES_TEST_URL to its own service, so it starts nothing there.
set -euo pipefail
[[ -z "${POSTGRES_TEST_URL:-}" ]] || exit 0
# without docker only the database tests fail, with the fixture's missing-URL error, and the rest of the run goes on
command -v docker > /dev/null || {
  echo "docker is not installed and POSTGRES_TEST_URL is not set, so the database tests will fail" >&2
  exit 0
}
image=postgres:16.6@sha256:557fea37a744d5f4c8faab304b0a90858b53ab119735a88c131fd19dab802f36
id=$(docker run --detach --rm --publish 127.0.0.1::5432 --tmpfs /var/lib/postgresql/data \
  --env POSTGRES_USER=fixture --env POSTGRES_PASSWORD=fixture --env POSTGRES_DB=postgres \
  "${image}" -c fsync=off -c synchronous_commit=off -c full_page_writes=off -c max_connections=500)
# the server goes away with the nextest process that started this script, however that process ends
setsid -f bash -c "tail --pid=${PPID} -f /dev/null; docker rm --force ${id}" < /dev/null > /dev/null 2>&1
address=$(docker port "${id}" 5432/tcp)
# the image's first start runs a server on the unix socket only while it initializes, so readiness is asked over TCP
SECONDS=0
while ((SECONDS < 30)); do
  if docker exec "${id}" pg_isready --quiet --host 127.0.0.1 --username fixture --dbname postgres; then
    echo "POSTGRES_TEST_URL=postgresql://fixture:fixture@${address}/postgres" >> "${NEXTEST_ENV:?}"
    exit 0
  fi
  sleep 0.1
done
echo "PostgreSQL in container ${id} did not accept TCP connections within 30 s" >&2
exit 1
