Postgres fixtures own independent databases and never seed through product repositories.
An explicit server URL takes precedence over testcontainers; database mode needs CREATEDB.
`PostgresFixture::provision_schema` instead creates an empty schema in that database.
Its connection options set search_path; it shares ProvisionTask cancellation and close semantics.
Use database mode for the pinned baseline, whose constraint guards are database-wide.

Async callers retain the `ProvisionTask` returned by `provision_with_name_generator`
and await `resolve()` to claim the fixture. If that future is cancelled, await
`ProvisionTask::close()` before shutting down the runtime. It drains the owned
`JoinSet`, closes unclaimed fixtures, and returns setup or cleanup errors. Dropping
the result future does not abort the database operation or detach a worker.

After a successful resolve, stop gateway processes and close their database clients
before calling `PostgresFixture::close_async()` (or `close()` for synchronous callers).
Only a successful CREATE DATABASE authorizes cleanup of its generated name; a name
collision must leave the pre-existing database intact.
