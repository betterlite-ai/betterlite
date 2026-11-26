pub mod background;
pub mod cache;
pub mod cached;
pub mod error;
pub mod health_state;
pub mod lease;
pub mod manager;
pub mod probe;
pub mod results;
pub mod route;
pub mod settings;
pub mod status_view;

#[path = "health_shared/tests/background_loop.rs"]
mod background_loop;
#[path = "health_shared/tests/cache_exceptions.rs"]
mod cache_exceptions;
#[path = "health_shared/tests/cache_get.rs"]
mod cache_get;
#[path = "health_shared/tests/cache_set.rs"]
mod cache_set;
#[path = "health_shared/tests/endpoint_auth.rs"]
mod endpoint_auth;
#[path = "health_shared/tests/error_handling.rs"]
mod error_handling;
#[path = "health_shared/tests/fixtures.rs"]
mod fixtures;
#[path = "health_shared/tests/key_builders.rs"]
mod key_builders;
#[path = "health_shared/tests/lock_acquire.rs"]
mod lock_acquire;
#[path = "health_shared/tests/lock_release.rs"]
mod lock_release;
#[path = "health_shared/tests/manager_init.rs"]
mod manager_init;
#[path = "health_shared/tests/no_redis.rs"]
mod no_redis;
#[path = "health_shared/tests/orchestration.rs"]
mod orchestration;
#[path = "health_shared/tests/pod_id.rs"]
mod pod_id;
#[path = "health_shared/tests/polling.rs"]
mod polling;
#[path = "health_shared/tests/status.rs"]
mod status;
#[path = "health_shared/tests/support.rs"]
mod support;
