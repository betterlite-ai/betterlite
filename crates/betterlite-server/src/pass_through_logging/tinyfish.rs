pub mod billing;
pub mod error;
pub mod fetch;
pub mod payload;
pub mod poll;
pub mod records;
pub mod settings;
pub mod sse;

use std::time::Duration;

use betterlite_config::loader::startup::process_environment;
use betterlite_http::user_url::split::urlparse;
use rust_decimal::Decimal;

pub const TINYFISH_DEFAULT_API_BASE: &str = "https://agent.tinyfish.ai";
pub const TINYFISH_AGENT_DOCS_URL: &str = "https://docs.tinyfish.ai/agent-api";
// TinyFish's published Agent API rate in USD per run step; env TINYFISH_COST_PER_STEP overrides it
pub const TINYFISH_DEFAULT_STEP_COST: Decimal = Decimal::from_parts(16, 0, 0, false, 3);
pub const TINYFISH_MODEL_NAME: &str = "tinyfish/automation-run";
// LiteLLM writes the poll interval and deadline as float seconds; whole seconds stay an exact Duration here.
pub const TINYFISH_POLL_SECONDS: Duration = Duration::from_secs(5);
pub const TINYFISH_MAX_POLL_SECONDS: Duration = Duration::from_secs(3600);
pub const TINYFISH_MAX_POLL_FAILURES: u32 = 12;
pub const TINYFISH_TERMINAL_STATUSES: [&str; 3] = ["COMPLETED", "FAILED", "CANCELLED"];
// these fields use the shared account's saved logins, so B3626 refuses them unless the run opt-in is set
pub const TINYFISH_RUN_AUTH_FIELDS: [&str; 4] = [
    "use_profile",
    "profile_id",
    "use_vault",
    "credential_item_ids",
];
// LiteLLM's pass-through envelope controls, which B3626 rejects before a run body forwards
pub const TINYFISH_ENVELOPE_FIELDS: [&str; 3] = ["custom_body", "stream", "query_params"];
// covers the upstream 1200s max run duration plus response headroom for blocking runs
pub const TINYFISH_TIMEOUT_SECONDS: f64 = 1500.0;

pub(crate) const TINYFISH_PROVIDER: &str = "tinyfish";
// A poller cancelled before it writes its spend row leaves the run unbilled, so the tracker warns with these words.
pub(crate) const BILLING_CANCELLED_WARNING: &str =
    "TinyFish passthrough: billing poller cancelled mid-poll; the run may go unbilled";

#[must_use]
pub fn resolve_tinyfish_api_base() -> String {
    let environment = process_environment();
    let configured = environment
        .get(settings::API_BASE_VARIABLE)
        .map(String::as_str);
    settings::api_base_from_text(configured)
}

#[must_use]
pub fn resolve_tinyfish_cost_per_step() -> Decimal {
    let environment = process_environment();
    let configured = environment.get(settings::COST_VARIABLE).map(String::as_str);
    settings::cost_per_step_from_text(configured)
}

// The host also serves vault/wallet/profile management under the same key, so only run endpoints forward.
#[must_use]
pub fn is_allowed_tinyfish_endpoint(method: &str, path: &str) -> bool {
    if !path.starts_with('/') {
        return false;
    }
    let segments = path.split('/').skip(1).collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| matches!(*segment, "" | "." | ".."))
    {
        return false;
    }
    match method {
        "POST" => matches!(
            segments.as_slice(),
            ["v1", "automation", "run" | "run-async" | "run-sse"] | ["v1", "runs", _, "cancel"]
        ),
        // no GET /v1/runs listing: run ids are unguessable, so the list stays closed to other teams
        "GET" => matches!(segments.as_slice(), ["v1", "runs", _]),
        _ => false,
    }
}

// LiteLLM compares the request URL's host against the resolved agent base, and a host-less base never matches.
#[must_use]
pub fn is_tinyfish_agent_url(url: &str) -> bool {
    let Ok(seen) = urlparse(url, "") else {
        return false;
    };
    let Some(hostname) = seen.hostname() else {
        return false;
    };
    let base = resolve_tinyfish_api_base();
    let agent = urlparse(&base, "")
        .ok()
        .and_then(|parsed| parsed.hostname());
    agent == Some(hostname)
}

// Only run submissions are billed; GET /v1/runs* polling and cancels never write spend rows.
#[must_use]
pub fn should_log_tinyfish_request(method: &str, url_route: &str) -> bool {
    let Ok(parsed) = urlparse(url_route, "") else {
        return false;
    };
    method == "POST" && parsed.path.contains("/v1/automation/")
}

#[must_use]
pub fn is_tinyfish_async_route(url_route: &str) -> bool {
    let Ok(parsed) = urlparse(url_route, "") else {
        return false;
    };
    parsed.path.ends_with("/v1/automation/run-async")
}

#[path = "tinyfish/tests/api_base_resolution.rs"]
mod api_base_resolution;
#[path = "tinyfish/tests/async_polling.rs"]
mod async_polling;
#[path = "tinyfish/tests/async_route.rs"]
mod async_route;
#[path = "tinyfish/tests/background_task.rs"]
mod background_task;
#[path = "tinyfish/tests/blocking_run.rs"]
mod blocking_run;
#[path = "tinyfish/tests/cost_resolution.rs"]
mod cost_resolution;
#[path = "tinyfish/tests/endpoint_validation.rs"]
mod endpoint_validation;
#[path = "tinyfish/tests/exception_handling.rs"]
mod exception_handling;
#[path = "tinyfish/tests/fetch_request.rs"]
mod fetch_request;
#[path = "tinyfish/tests/hostname.rs"]
mod hostname;
#[path = "tinyfish/tests/missing_runid.rs"]
mod missing_runid;
#[path = "tinyfish/tests/parse_run_billing.rs"]
mod parse_run_billing;
#[path = "tinyfish/tests/payload_preservation.rs"]
mod payload_preservation;
#[path = "tinyfish/tests/polling_budget.rs"]
mod polling_budget;
#[path = "tinyfish/tests/request_logging.rs"]
mod request_logging;
#[path = "tinyfish/tests/run_cost.rs"]
mod run_cost;
#[path = "tinyfish/tests/run_parsing.rs"]
mod run_parsing;
#[path = "tinyfish/tests/runid_validation.rs"]
mod runid_validation;
#[path = "tinyfish/tests/sse_extraction.rs"]
mod sse_extraction;
#[path = "tinyfish/tests/sse_fallback.rs"]
mod sse_fallback;
#[path = "tinyfish/tests/sse_polling.rs"]
mod sse_polling;
#[path = "tinyfish/tests/support.rs"]
mod support;
#[path = "tinyfish/tests/task_registration.rs"]
mod task_registration;
#[path = "tinyfish/tests/terminal_detection.rs"]
mod terminal_detection;
