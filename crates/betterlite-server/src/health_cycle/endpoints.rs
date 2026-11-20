use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use serde_json::Number;

use crate::health::router::deployment_id;
use crate::health_cycle::ports::{ProbeFailure, ProbeResponse};
use crate::health_cycle::probe::ProbeOutcome;
use crate::health_probe_settings::display::clean_endpoint_data;

// _perform_health_check's three answers: the display rows, and the live exceptions kept off the JSON for shared-cache readers
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HealthCheckResult {
    pub healthy_endpoints: Vec<JsonObject>,
    pub unhealthy_endpoints: Vec<JsonObject>,
    pub exceptions_by_model_id: BTreeMap<String, ProbeFailure>,
}

// The id a row was filed under, non-empty text only, as `if model_id:` reads it
pub(crate) fn endpoint_id(endpoint: &JsonObject) -> Option<&str> {
    match endpoint.get("model_id") {
        Some(Json::Text(id)) => (!id.is_empty()).then_some(id.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// {**litellm_params, **result}, filtered to the approved display fields before the id is added
fn endpoint_json(deployment: &DeploymentSpec, result: &JsonObject, details: bool) -> JsonObject {
    let mut merged = deployment.litellm_params.clone();
    for (key, value) in result {
        merged.insert(key.clone(), value.clone());
    }
    clean_endpoint_data(&merged, details)
}

// _create_health_check_response: the rate-limit headers a healthy provider answered, each kept only when it was sent
fn response_json(response: &ProbeResponse) -> JsonObject {
    let mut headers = JsonObject::new();
    for (key, value) in [
        (
            "x-ratelimit-remaining-requests",
            &response.remaining_requests,
        ),
        ("x-ratelimit-remaining-tokens", &response.remaining_tokens),
        ("x-ms-region", &response.region),
    ] {
        if let Some(value) = value {
            headers.insert(key.to_owned(), Json::Text(value.clone()));
        }
    }
    headers
}

// The exception object never enters the JSON: its status becomes the integer a shared cache can reconstruct the filter from
fn separate_one(
    deployment: &DeploymentSpec,
    outcome: &ProbeOutcome,
    details: bool,
    result: &mut HealthCheckResult,
) {
    let (mut row, failure) = match outcome {
        ProbeOutcome::Healthy(response) => {
            let mut healthy = endpoint_json(deployment, &response_json(response), details);
            if let Some(model_id) = deployment_id(deployment) {
                healthy.insert("model_id".to_owned(), Json::Text(model_id.to_owned()));
            }
            result.healthy_endpoints.push(healthy);
            return;
        }
        ProbeOutcome::Failed(failure) => (
            endpoint_json(deployment, &failure.to_json(), details),
            failure,
        ),
        ProbeOutcome::Raised(failure) => (
            endpoint_json(deployment, &JsonObject::new(), details),
            failure,
        ),
    };
    if let Some(model_id) = deployment_id(deployment) {
        row.insert("model_id".to_owned(), Json::Text(model_id.to_owned()));
        row.insert(
            "exception_status".to_owned(),
            Json::Number(Number::from(u64::from(failure.status_code))),
        );
        result
            .exceptions_by_model_id
            .insert(model_id.to_owned(), failure.clone());
    }
    result.unhealthy_endpoints.push(row);
}

// _perform_health_check's loop: a dict without "error" is healthy, and a failed probe is filed under its model id when it has one
#[must_use]
pub fn separate_results(
    checked: &[DeploymentSpec],
    outcomes: &[ProbeOutcome],
    details: bool,
) -> HealthCheckResult {
    let mut result = HealthCheckResult::default();
    for (deployment, outcome) in checked.iter().zip(outcomes) {
        separate_one(deployment, outcome, details, &mut result);
    }
    result
}
