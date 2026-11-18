Postgres fixtures own independent databases and never seed through product repositories.
POSTGRES_TEST_URL names the server: CI sets it, and cargo nextest starts one per run through .config/nextest.toml. The user needs CREATEDB.
Each database is a copy of a template that holds its schema, built once per server and schema.
`PostgresFixture::provision_with_statements` copies a template built from the statements it is given, and an empty list gives an empty database. Its tables live in public, and it shares ProvisionTask cancellation and close semantics.
TLS tests generate their certificates at test time and front the configured server, so no certificate or key is checked in.

Async callers retain the `ProvisionTask` returned by `provision_with_name_generator`
and await `resolve()` to claim the fixture. If that future is cancelled, await
`ProvisionTask::close()` before shutting down the runtime. It drains the owned
`JoinSet`, closes unclaimed fixtures, and returns setup or cleanup errors. Dropping
the result future does not abort the database operation or detach a worker.

After a successful resolve, stop gateway processes and close their database clients
before calling `PostgresFixture::close_async()` (or `close()` for synchronous callers).
Only a successful CREATE DATABASE authorizes cleanup of its generated name; a name
collision must leave the pre-existing database intact.
