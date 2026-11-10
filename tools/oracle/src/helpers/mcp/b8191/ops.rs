use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Mutex, PoisonError};

use betterlite_mcp::request_context::allowlist::{
    check_mcp_client_allowed, load_mcp_client_allowlist,
};
use betterlite_mcp::request_context::elicitation::{
    DownstreamElicitSession, ElicitRequestParams, ElicitResult, handle_elicitation_request,
};
use http::HeaderMap;
use http::HeaderName;
use http::header::HeaderValue;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::{HelperError, HelperResult};

use super::input::{
    AllowlistLoadInput, CheckScenario, ElicitationInput, ElicitationScenario, Input,
    SessionBehaviour,
};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::AllowlistLoad(load) => Ok(allowlist_load_value(load)),
        Input::AllowlistCheck(check) => Ok(json!({
            "results": check.scenarios.iter().map(check_value).collect::<Vec<_>>(),
        })),
        Input::Elicitation(elicitation) => elicitation_value(elicitation),
    }
}

fn allowlist_load_value(input: &AllowlistLoadInput) -> Value {
    match load_mcp_client_allowlist(&input.settings) {
        None => json!({ "enforced": false }),
        Some(allowlist) => json!({
            "enforced": true,
            "aliases_by_value": allowlist.aliases_by_value,
            "jwt_field": allowlist.jwt_field,
            "header": allowlist.header,
        }),
    }
}

fn check_value(scenario: &CheckScenario) -> Value {
    let headers = header_map(&scenario.headers);
    match check_mcp_client_allowed(
        scenario.allowlist.as_ref(),
        scenario.jwt_claims.as_ref(),
        &headers,
    ) {
        Ok(()) => json!({ "allowed": true }),
        Err(rejection) => {
            let body = rejection.response_body();
            json!({
                "allowed": false,
                "details": rejection.details,
                "body": body,
            })
        }
    }
}

fn header_map(headers: &BTreeMap<String, String>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (header_name, header_value) in headers {
        let Ok(name) = HeaderName::from_bytes(header_name.as_bytes()) else {
            continue;
        };
        let Some(value) = latin1_header_value(header_value) else {
            continue;
        };
        map.insert(name, value);
    }
    map
}

fn latin1_header_value(text: &str) -> Option<HeaderValue> {
    let bytes = text
        .chars()
        .map(|character| u8::try_from(u32::from(character)).ok())
        .collect::<Option<Vec<u8>>>()?;
    HeaderValue::from_bytes(&bytes).ok()
}

fn elicitation_value(input: &ElicitationInput) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Box::new(failure("RuntimeError", error.to_string())))?;
    let results = input
        .scenarios
        .iter()
        .map(|scenario| elicitation_scenario(&runtime, scenario))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "results": results }))
}

fn elicitation_scenario(
    runtime: &tokio::runtime::Runtime,
    scenario: &ElicitationScenario,
) -> Result<Value, Box<HelperError>> {
    let session = scenario.session.clone().map(|behaviour| RecordingSession {
        behaviour,
        calls: Mutex::new(Vec::new()),
    });
    let outcome = runtime.block_on(handle_elicitation_request(
        &scenario.params,
        session.as_ref(),
        scenario.capabilities.as_ref(),
    ));
    let mut value = serde_json::to_value(&outcome)
        .map_err(|error| Box::new(failure("SerializationError", error.to_string())))?;
    if let Value::Object(object) = &mut value {
        let calls = session
            .map(|recording| recording.recorded())
            .unwrap_or_default();
        object.insert("calls".to_owned(), Value::Array(calls));
    }
    Ok(value)
}

fn failure(class: &str, message: String) -> HelperError {
    HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}

struct RecordingSession {
    behaviour: SessionBehaviour,
    calls: Mutex<Vec<Value>>,
}

impl RecordingSession {
    fn recorded(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, Error)]
#[error("relay failed: {0}")]
struct RelayFailure(String);

impl DownstreamElicitSession for RecordingSession {
    type Error = RelayFailure;

    fn elicit(
        &self,
        params: &ElicitRequestParams,
    ) -> impl Future<Output = Result<ElicitResult, RelayFailure>> + Send {
        let recorded = match params {
            ElicitRequestParams::Url(url) => json!({
                "method": "elicit_url",
                "message": url.message,
                "url": url.url,
                "elicitation_id": url.elicitation_id,
            }),
            ElicitRequestParams::Form(form) => json!({
                "method": "elicit_form",
                "message": form.message,
                "requested_schema": form.requested_schema,
            }),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        let behaviour = self.behaviour.clone();
        async move {
            match behaviour {
                SessionBehaviour::Result { result } => Ok(result),
                SessionBehaviour::Error { error } => Err(RelayFailure(error)),
            }
        }
    }
}
