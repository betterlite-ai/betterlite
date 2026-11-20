pub mod catalog;
pub mod concurrency;
pub mod cycle;
pub mod dispatch;
pub mod endpoints;
pub mod error;
pub mod ports;
pub mod probe;
pub mod rss;
pub mod state;
pub mod telemetry;
pub mod verdict;

#[path = "health_cycle/tests/bounded_concurrency.rs"]
mod bounded_concurrency;
#[path = "health_cycle/tests/cli_fallback.rs"]
mod cli_fallback;
#[path = "health_cycle/tests/eligibility.rs"]
mod eligibility;
#[path = "health_cycle/tests/health_state.rs"]
mod health_state;
#[path = "health_cycle/tests/instrumentation.rs"]
mod instrumentation;
#[path = "health_cycle/tests/model_probe.rs"]
mod model_probe;
#[path = "health_cycle/tests/orchestration.rs"]
mod orchestration;
#[path = "health_cycle/tests/perform_health.rs"]
mod perform_health;
#[path = "health_cycle/tests/process_rss.rs"]
mod process_rss;
#[path = "health_cycle/tests/rss_formatting.rs"]
mod rss_formatting;
#[path = "health_cycle/tests/support.rs"]
mod support;
#[path = "health_cycle/tests/targeting.rs"]
mod targeting;
#[path = "health_cycle/tests/timeout.rs"]
mod timeout;
