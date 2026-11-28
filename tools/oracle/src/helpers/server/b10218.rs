use std::collections::BTreeMap;
use std::panic::Location;

use betterlite_auth::known_proxy_routes::{known_proxy_route, known_proxy_routes};
use betterlite_config::loader::ConfigLoader;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_reporting::bug_report::notice::bug_report_notice;
use betterlite_reporting::bug_report::record::BugReport;
use betterlite_reporting::safe_proxy_config::ProxyCall;
use betterlite_server::api::proxy_endpoint_support::FailureFacts;
use betterlite_server::state::proxy_diagnostics::Slot;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    // the case env again, since the Rust side of a helper sees only its input
    env: BTreeMap<String, String>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Notice { record: BugReport },
    Failure { status_code: Option<u16> },
    Routes { excluded: Vec<String> },
    Route { path: String },
}

// the case env through the config loader, then B10058's record for a failure the caller built
fn failure_answer(
    status_code: Option<u16>,
    env: &BTreeMap<String, String>,
) -> Result<Value, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("{}")
    .or_config_error()?;
    let slot = Slot::new(&config);
    let failure = FailureFacts {
        message: "boom",
        exception_type: "FailureError",
        status_code,
        param: None,
        database_error: None,
        // a tools path, so the record keeps no frame, as a LiteLLM error that was never raised keeps none
        location: Location::caller(),
    };
    let report = slot.notices.report(&failure, ProxyCall::default());
    // None goes out as null, the answer the Python side gives when should_report_bug is false
    Ok(serde_json::to_value(report)?)
}

// B992's claude-code plugin routes sit in betterlite's anthropic table but not in LiteLLM's, so the case excludes them on both sides
fn routes_answer(excluded: &[String]) -> Value {
    let routes = known_proxy_routes()
        .into_iter()
        .filter(|route| !excluded.iter().any(|skip| skip == route))
        .collect::<Vec<&str>>();
    json!(routes)
}

fn answer(call: &Call, env: &BTreeMap<String, String>) -> Result<Value, Box<HelperError>> {
    match call {
        // B4170's notice for a record the case pins whole, so both sides build the same URL
        Call::Notice { record } => Ok(Value::String(bug_report_notice(record))),
        Call::Failure { status_code } => failure_answer(*status_code, env),
        Call::Routes { excluded } => Ok(routes_answer(excluded)),
        Call::Route { path } => Ok(json!(known_proxy_route(path))),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input
            .calls
            .iter()
            .map(|call| answer(call, &input.env))
            .collect::<Result<Vec<Value>, _>>()?;
        Ok(json!({ "calls": answers }))
    }
}
