use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::responses_id_security::error::{Forbidden, ResponsesIdSecurityError};
use betterlite_core::api::responses_id_security::{
    ADDRESSED_RESPONSE_ID_FIELD, CallOutcome, ResponsesCall,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::values::{EventView, nested_id, project, project_text, view_event};
use super::{Cipher, PrincipalInput, SettingsInput, failure, principal, session};

pub(crate) fn authorize(
    settings: &SettingsInput,
    principal_input: &PrincipalInput,
    call_type: &str,
    data: &IndexMap<String, Json>,
) -> HelperResult {
    let state = session(settings);
    let caller = principal(principal_input)?;
    let mut payload = data.clone();
    let call = ResponsesCall::from_call_type(call_type);
    let outcome = state.policy().authorize_call(call, &mut payload, &caller);
    let field_name = addressed_field(call_type);
    let answer = call_answer(&payload, field_name, &outcome, &state.cipher)?;
    Ok(json!({ "call": answer }))
}

pub(crate) fn success(
    settings: &SettingsInput,
    principal_input: &PrincipalInput,
    response: &ResponsesApiResponse,
) -> HelperResult {
    let state = session(settings);
    let caller = principal(principal_input)?;
    let mut body = response.clone();
    state
        .policy()
        .encrypt_response(&mut body, &caller)
        .map_err(|error| internal_failure(&error))?;
    let nested = body
        .extra
        .get("response")
        .map(|value| project(nested_id(value), &state.cipher))
        .transpose()?;
    let id = project_text(&body.id, &state.cipher);
    Ok(json!({ "response": { "id": id, "nested": nested } }))
}

pub(crate) fn stream(
    settings: &SettingsInput,
    principal_input: &PrincipalInput,
    route: Option<&str>,
    events: &[ResponsesAPIStreamingResponse],
) -> HelperResult {
    let caller = principal(principal_input)?;
    let first = run_stream(settings, &caller, route, events)?;
    let second = run_stream(settings, &caller, route, events)?;
    Ok(json!({ "stream": {
        "events": first.views,
        "shared": shared_advertisement(&first.advertised),
        "second_request_differs": first.raw != second.raw,
    } }))
}

struct StreamRun {
    views: Vec<Value>,
    raw: Vec<String>,
    advertised: Vec<Option<String>>,
}

fn run_stream(
    settings: &SettingsInput,
    caller: &Principal,
    route: Option<&str>,
    events: &[ResponsesAPIStreamingResponse],
) -> Result<StreamRun, Box<HelperError>> {
    let state = session(settings);
    let policy = state.policy();
    let mut cache = BTreeMap::new();
    let mut views = Vec::new();
    let mut raw = Vec::new();
    let mut advertised = Vec::new();
    for event in events {
        let mut chunk = event.clone();
        policy
            .rewrite_stream_chunk(route, &mut chunk, caller, &mut cache)
            .map_err(|error| internal_failure(&error))?;
        let serialized = serde_json::to_value(&chunk)
            .map_err(|error| failure("SerializeFailed", error.to_string()))?;
        let EventView {
            top,
            nested,
            advertised: event_id,
        } = view_event(&serialized, &state.cipher)?;
        views.push(json!({ "top": top, "nested": nested }));
        advertised.push(event_id);
        let text = serde_json::to_string(&serialized)
            .map_err(|error| failure("SerializeFailed", error.to_string()))?;
        raw.push(text);
    }
    Ok(StreamRun {
        views,
        raw,
        advertised,
    })
}

// the events of one stream all advertise the same ciphertext for one original id
fn shared_advertisement(advertised: &[Option<String>]) -> bool {
    match advertised {
        [Some(first), rest @ ..] => rest.iter().all(|value| value.as_ref() == Some(first)),
        [] | [None, ..] => false,
    }
}

fn call_answer(
    data: &IndexMap<String, Json>,
    field_name: Option<&str>,
    outcome: &Result<CallOutcome, ResponsesIdSecurityError>,
    cipher: &Cipher,
) -> Result<Value, Box<HelperError>> {
    let (kind, detail, status, class) = match outcome {
        Ok(CallOutcome::NotResponsesCall) => ("not_responses_call", None, None, None),
        Ok(CallOutcome::Unchanged) => ("unchanged", None, None, None),
        Ok(CallOutcome::Authorized { .. }) => ("authorized", None, None, None),
        Err(ResponsesIdSecurityError::Forbidden(forbidden)) => (
            forbidden_kind(*forbidden),
            Some(forbidden.detail()),
            Some(403_i64),
            None,
        ),
        Err(error) => (
            "internal_error",
            None,
            Some(i64::from(error.status_code())),
            Some(internal_class(error)),
        ),
    };
    Ok(json!({
        "kind": kind,
        "detail": detail,
        "type": Value::Null,
        "param": Value::Null,
        "code": Value::Null,
        "status": status,
        "field": project_field(data, field_name, cipher)?,
        "retained": project_field(data, Some(ADDRESSED_RESPONSE_ID_FIELD), cipher)?,
        "class": class,
    }))
}

fn project_field(
    data: &IndexMap<String, Json>,
    field_name: Option<&str>,
    cipher: &Cipher,
) -> Result<Option<String>, Box<HelperError>> {
    match field_name.and_then(|name| data.get(name)) {
        Some(value) => Ok(Some(project(value, cipher)?)),
        None => Ok(None),
    }
}

fn addressed_field(call_type: &str) -> Option<&'static str> {
    match call_type {
        "aresponses" => Some("previous_response_id"),
        "aget_responses" | "adelete_responses" | "acancel_responses" | "alist_input_items" => {
            Some("response_id")
        }
        _ => None,
    }
}

const fn forbidden_kind(forbidden: Forbidden) -> &'static str {
    match forbidden {
        Forbidden::Unmanaged => "unmanaged_forbidden",
        Forbidden::User => "user_forbidden",
        Forbidden::Team => "team_forbidden",
    }
}

const fn internal_class(error: &ResponsesIdSecurityError) -> &'static str {
    match error {
        ResponsesIdSecurityError::Forbidden(_) => "Forbidden",
        ResponsesIdSecurityError::NotUtf8 => "NotUtf8",
        ResponsesIdSecurityError::MalformedOwnedPayload { .. } => "MalformedOwnedPayload",
        ResponsesIdSecurityError::NonTextCiphertext => "NonTextCiphertext",
        ResponsesIdSecurityError::Encrypt { .. } => "EncryptFailed",
    }
}

fn internal_failure(error: &ResponsesIdSecurityError) -> Box<HelperError> {
    Box::new(HelperError {
        class: internal_class(error).to_owned(),
        status: Some(error.status_code()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
