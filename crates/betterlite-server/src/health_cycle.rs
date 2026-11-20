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

#[cfg(test)]
#[path = "tests/support.rs"]
mod support;
#[cfg(test)]
#[path = "tests/process_rss.rs"]
mod process_rss;
#[cfg(test)]
#[path = "tests/rss_formatting.rs"]
mod rss_formatting;
#[cfg(test)]
#[path = "tests/model_probe.rs"]
mod model_probe;
#[cfg(test)]
#[path = "tests/bounded_concurrency.rs"]
mod bounded_concurrency;
#[cfg(test)]
#[path = "tests/perform_health.rs"]
mod perform_health;
#[cfg(test)]
#[path = "tests/health_state.rs"]
mod health_state;
#[cfg(test)]
#[path = "tests/orchestration.rs"]
mod orchestration;
#[cfg(test)]
#[path = "tests/targeting.rs"]
mod targeting;
#[cfg(test)]
#[path = "tests/eligibility.rs"]
mod eligibility;
#[cfg(test)]
#[path = "tests/timeout.rs"]
mod timeout;
#[cfg(test)]
#[path = "tests/cli_fallback.rs"]
mod cli_fallback;
#[cfg(test)]
#[path = "tests/instrumentation.rs"]
mod instrumentation;
