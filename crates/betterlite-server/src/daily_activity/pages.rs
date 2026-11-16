pub mod aggregate;
pub mod breakdown;
pub mod error;
pub mod metadata;
pub mod read;
pub mod record;

#[path = "pages/tests/async_aggregate.rs"]
mod async_aggregate;
#[path = "pages/tests/metadata_callback.rs"]
mod metadata_callback;
#[path = "pages/tests/model_group.rs"]
mod model_group;
#[path = "pages/tests/pagination.rs"]
mod pagination;
#[path = "pages/tests/read_service.rs"]
mod read_service;
#[path = "pages/tests/sentinel.rs"]
mod sentinel;
#[path = "pages/tests/support.rs"]
mod support;
#[path = "pages/tests/sync_aggregate.rs"]
mod sync_aggregate;
