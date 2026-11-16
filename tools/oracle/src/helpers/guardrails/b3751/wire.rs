use std::collections::BTreeMap;

use betterlite_guardrails::akto::AktoMirroring;
use betterlite_guardrails::akto::error::AktoError;
use betterlite_guardrails::akto::payload::{AktoPayload, build_akto_payload};
use betterlite_guardrails::akto::transport::{
    AktoAnswer, AktoCallFlags, fire_and_forget_request, handle_guardrail_response, send_request,
};
use serde_json::{Value, json};

use super::Input;
use super::akto_inputs;
use super::failure;
use super::inputs_json;
use super::settings;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_oracle_runner::mock::{MockProvider, ProviderRequest};
use betterlite_oracle_runner::routing_mock_scripts::{MockScript, ProviderCall};
use std::slice;
use tokio::runtime::Builder;
use url::form_urlencoded;

pub(super) fn build_payload(input: &Input) -> HelperResult {
    let guardrail = settings(input)?;
    let payload = payload_of(input, &guardrail)?;
    normalize(payload)
}

// the wall clock is compared by shape, not by value, so a stamped time reads as a placeholder
fn normalized_time(time: String) -> String {
    if time.chars().all(|character| character.is_ascii_digit()) {
        return String::from("<millis>");
    }
    time
}

fn payload_of(input: &Input, guardrail: &AktoMirroring) -> Result<AktoPayload, Box<HelperError>> {
    build_akto_payload(
        guardrail,
        &akto_inputs(input),
        &input.request_data,
        input.status_code.unwrap_or(200),
        input.include_response,
    )
    .map_err(|error| failure("AktoError", None, error.to_string()))
}

pub(super) fn normalize(payload: AktoPayload) -> HelperResult {
    let request_headers: Value = serde_json::from_str(&payload.request_headers)
        .map_err(|error| failure("JsonError", None, error.to_string()))?;
    let response_headers: Value = serde_json::from_str(&payload.response_headers)
        .map_err(|error| failure("JsonError", None, error.to_string()))?;
    let request_payload = decode_encoded(&payload.request_payload, false)?;
    let response_payload = decode_encoded(&payload.response_payload, true)?;
    let tag: Value = serde_json::from_str(&payload.tag)
        .map_err(|error| failure("JsonError", None, error.to_string()))?;
    let metadata: Value = serde_json::from_str(&payload.metadata)
        .map_err(|error| failure("JsonError", None, error.to_string()))?;
    Ok(json!({
        "path": payload.path,
        "requestHeaders": request_headers,
        "responseHeaders": response_headers,
        "method": payload.method,
        "requestPayload": request_payload,
        "responsePayload": response_payload,
        "ip": payload.ip,
        "destIp": payload.dest_ip,
        "time": normalized_time(payload.time),
        "statusCode": payload.status_code,
        "type": payload.request_type,
        "status": payload.status,
        "akto_account_id": payload.akto_account_id,
        "akto_vxlan_id": payload.akto_vxlan_id,
        "is_pending": payload.is_pending,
        "source": payload.source,
        "direction": payload.direction,
        "process_id": payload.process_id,
        "socket_id": payload.socket_id,
        "daemonset_id": payload.daemonset_id,
        "enabled_graph": payload.enabled_graph,
        "tag": tag,
        "metadata": metadata,
        "contextSource": payload.context_source,
    }))
}

// the request payload is always double encoded; the empty response payload is a bare object
fn decode_encoded(encoded: &str, response_payload: bool) -> Result<Value, Box<HelperError>> {
    let outer: Value = serde_json::from_str(encoded)
        .map_err(|error| failure("JsonError", None, error.to_string()))?;
    if response_payload && outer.get("body").is_none() {
        return Ok(outer);
    }
    let Some(Value::String(inner)) = outer.get("body") else {
        return Err(failure(
            "AktoError",
            None,
            "the encoded payload has no inner body".to_owned(),
        ));
    };
    serde_json::from_str(inner.as_str())
        .map_err(|error| failure("JsonError", None, error.to_string()))
}

pub(super) fn send(input: &Input, fire_and_forget: bool) -> HelperResult {
    let mut guardrail = settings(input)?;
    let mock = MockProvider::start(script(&input.answer)?)
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    guardrail.base_url.clone_from(&mock.base_url);
    let payload = payload_of(input, &guardrail)?;
    let answer = deliver(&guardrail, input, &payload, fire_and_forget)?;
    let calls = mock
        .finish()
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    report(&calls, answer)
}

pub(super) fn unreachable(input: &Input) -> HelperResult {
    let Err(error) =
        settings(input)?.handle_unreachable(akto_inputs(input), &AktoError::Unreachable)
    else {
        return Ok(json!({"passthrough": inputs_json(input)}));
    };
    Err(failure(
        "HTTPException",
        Some(error.status_code().as_u16()),
        error.to_string(),
    ))
}

fn deliver(
    guardrail: &AktoMirroring,
    input: &Input,
    payload: &AktoPayload,
    fire_and_forget: bool,
) -> Result<Option<Result<AktoAnswer, AktoError>>, Box<HelperError>> {
    let flags = AktoCallFlags {
        guardrails: input.guardrails.unwrap_or(false),
        ingest_data: input.ingest_data.unwrap_or(false),
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", None, error.to_string()))?;
    Ok(runtime.block_on(async {
        if fire_and_forget {
            fire_and_forget_request(guardrail, flags, payload).await;
            None
        } else {
            Some(send_request(guardrail, flags, payload).await)
        }
    }))
}

fn report(calls: &[ProviderCall], answer: Option<Result<AktoAnswer, AktoError>>) -> HelperResult {
    let request = calls
        .first()
        .ok_or_else(|| failure("MockError", None, "no request reached the mock".to_owned()))
        .and_then(|call| captured_request(&call.request))?;
    match answer {
        None => Ok(json!({"request": request, "succeeded": true})),
        Some(Ok(received)) => {
            let verdict = handle_guardrail_response(&received)
                .map_err(|error| failure("AktoError", None, error.to_string()))?;
            Ok(json!({"request": request, "allowed": verdict.allowed, "reason": verdict.reason}))
        }
        Some(Err(error)) => Err(failure("AktoError", None, error.to_string())),
    }
}

fn script(answer: &super::AnswerInput) -> Result<MockScript, Box<HelperError>> {
    let reply = UpstreamAnswer {
        status: answer.status.unwrap_or(200),
        headers: BTreeMap::new(),
        body: Some(answer.body.clone().unwrap_or(Value::Null)),
        events: None,
    };
    MockScript::try_from(slice::from_ref(&reply))
        .map_err(|error| failure("ScriptError", None, error.to_string()))
}

fn captured_request(request: &ProviderRequest) -> Result<Value, Box<HelperError>> {
    let (path, query) = match request.path.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (request.path.as_str(), None),
    };
    let mut params = serde_json::Map::new();
    if let Some(found) = query {
        for (name, value) in form_urlencoded::parse(found.as_bytes()) {
            params.insert(name.into_owned(), Value::String(value.into_owned()));
        }
    }
    let captured: AktoPayload = serde_json::from_value(request.body.clone())
        .map_err(|error| failure("JsonError", None, error.to_string()))?;
    let body = normalize(captured)?;
    Ok(json!({
        "path": path,
        "query": Value::Object(params),
        "content_type": request.headers.get("content-type"),
        "body": body,
    }))
}
