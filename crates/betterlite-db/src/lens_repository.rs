pub mod database;
pub mod repository;
mod sql;
pub mod update;

#[path = "lens_repository/tests/support.rs"]
mod support;

#[path = "lens_repository/tests/archive_displaced_jobs.rs"]
mod archive_displaced_jobs;

#[path = "lens_repository/tests/changed_only.rs"]
mod changed_only;

#[path = "lens_repository/tests/construction.rs"]
mod construction;

#[path = "lens_repository/tests/history_pages.rs"]
mod history_pages;

#[path = "lens_repository/tests/lens_create.rs"]
mod lens_create;

#[path = "lens_repository/tests/lens_queries.rs"]
mod lens_queries;

#[path = "lens_repository/tests/optimistic_update.rs"]
mod optimistic_update;

#[path = "lens_repository/tests/revocation_aware.rs"]
mod revocation_aware;

#[path = "lens_repository/tests/single_job.rs"]
mod single_job;

#[path = "lens_repository/tests/update_lens_missing.rs"]
mod update_lens_missing;

#[path = "lens_repository/tests/version_race.rs"]
mod version_race;

#[path = "lens_repository/tests/worker_heartbeat.rs"]
mod worker_heartbeat;

#[path = "lens_repository/tests/worker_queries.rs"]
mod worker_queries;

#[path = "lens_repository/tests/worker_revocation.rs"]
mod worker_revocation;

#[path = "lens_repository/tests/worker_save.rs"]
mod worker_save;
