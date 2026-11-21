use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_id_security::error::{Error, Forbidden};
use betterlite_core::api::responses_id_security::{
    ADDRESSED_RESPONSE_ID_FIELD, CallOutcome, ResponsesCall, ResponsesIdSecurity,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::values::{build_event, materialize, nested_json, project, project_nested, view_event};
use super::{
    Cipher, EventSpec, FieldInput, NestedSpec, PrincipalInput, SettingsInput, ValueSpec, failure,
    principal, session,
};

pub(crate) fn authorize(
    settings: &SettingsInput,
    principal_input: &PrincipalInput,
    call_type: &str,
    fields: &[FieldInput],
) -> HelperResult {
    let state = session(settings);
    let caller = principal(principal_input)?;
    let mut data = IndexMap::new();
    for field in fields {
        data.insert(
            field.field.clone(),
            materialize(&state.cipher, &field.value)?,
        );
    }
    let call = ResponsesCall::from_call_type(call_type);
    let policy = ResponsesIdSecurity {
        settings: &state.settings,
        cipher: &state.cipher,
        limits: ApiLimits::default(),
    };
    let outcome = policy.authorize_call(call, &mut data, &caller);
    let field_name = addressed_field(call_type);
    let answer = call_answer(&data, field_name, &outcome, &state.cipher)?;
    Ok(json!({ "call": answer }))
}

pub(crate) fn success(
    settings: &SettingsInput,
    principal_input: &PrincipalInput,
    id: &ValueSpec,
    nested: &NestedSpec,
) -> HelperResult {
    let state = session(settings);
    let caller = principal(principal_input)?;
    let Json::Text(response_id) = materialize(&state.cipher, id)? else {
        return Err(failure(
            "InvalidInput",
            "the response id must be a string".to_owned(),
        ));
    };
    let mut response: ResponsesApiResponse = serde_json::from_value(json!({
        "id": response_id,
        "created_at": 0,
        "output": [],
    }))
    .map_err(|error| failure("InvalidInput", error.to_string()))?;
    if let Some(value) = nested_json(&state.cipher, nested)? {
        response.extra.insert("response".to_owned(), value);
    }
    let policy = ResponsesIdSecurity {
        settings: &state.settings,
        cipher: &state.cipher,
        limits: ApiLimits::default(),
    };
    policy
        .encrypt_response(&mut response, &caller)
        .map_err(|error| internal_failure(&error))?;
    let id_out = project(&Json::Text(response.id.clone()), &state.cipher)?;
    let nested_out = match response.extra.get("response") {
        Some(value) => Some(project_nested(value, &state.cipher)?),
        None => None,
    };
    Ok(json!({ "response": { "id": id_out, "nested": nested_out } }))
}

pub(crate) fn stream(
    settings: &SettingsInput,
    principal_input: &PrincipalInput,
    route: Option<&str>,
    events: &[EventSpec],
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
    events: &[EventSpec],
) -> Result<StreamRun, Box<HelperError>> {
    let state = session(settings);
    let policy = ResponsesIdSecurity {
        settings: &state.settings,
        cipher: &state.cipher,
        limits: ApiLimits::default(),
    };
    let mut cache = BTreeMap::new();
    let mut views = Vec::new();
    let mut raw = Vec::new();
    let mut advertised = Vec::new();
    for spec in events {
        let (mut event, built) = build_event(&state.cipher, spec)?;
        policy
            .rewrite_stream_chunk(route, &mut event, caller, &mut cache)
            .map_err(|error| internal_failure(&error))?;
        let serialized = serde_json::to_value(&event)
            .map_err(|error| failure("SerializeFailed", error.to_string()))?;
        let view = view_event(&serialized, built, &state.cipher)?;
        views.push(json!({ "top": view.top, "nested": view.nested }));
        advertised.push(view.advertised);
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
    outcome: &Result<CallOutcome, Error>,
    cipher: &Cipher,
) -> Result<Value, Box<HelperError>> {
    let (kind, detail, status, class) = match outcome {
        Ok(CallOutcome::NotResponsesCall) => ("not_responses_call", None, None, None),
        Ok(CallOutcome::Unchanged) => ("unchanged", None, None, None),
        Ok(CallOutcome::Authorized { .. }) => ("authorized", None, None, None),
        Err(Error::Forbidden(forbidden)) => (
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

fn internal_class(error: &Error) -> &'static str {
    match error {
        Error::Forbidden(_) => "Forbidden",
        Error::NotUtf8 => "NotUtf8",
        Error::MalformedOwnedPayload { .. } => "MalformedOwnedPayload",
        Error::NonTextCiphertext => "NonTextCiphertext",
        Error::Encrypt { .. } => "EncryptFailed",
    }
}

fn internal_failure(error: &Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: internal_class(error).to_owned(),
        status: Some(error.status_code()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
