use std::collections::BTreeMap;

use betterlite_guardrails::akto::AktoMirroring;
use betterlite_guardrails::akto::error::AktoError;
use betterlite_guardrails::akto::payload::AktoPayload;
use betterlite_guardrails::akto::transport::AktoAnswer;
use serde_json::{Value, json};

use super::Input;
use super::failure;
use crate::case::UpstreamAnswer;
use crate::helpers::HelperError;
use crate::mock::MockProvider;
use crate::mock::ProviderRequest;
use crate::routing_mock_scripts::MockScript;
use crate::routing_mock_scripts::ProviderCall;

pub(super) fn build_payload(input: &Input) -> crate::helpers::HelperResult {
    let guardrail = super::settings(input)?;
    let payload = guardrail
        .build_akto_payload(
            &super::akto_inputs(input),
            &input.request_data,
            input.status_code.unwrap_or(200),
            input.include_response,
        )
        .map_err(|error| failure("AktoError", None, error.to_string()))?;
    normalize(payload)
}

pub(super) fn normalize(payload: AktoPayload) -> crate::helpers::HelperResult {
    let request_headers = parse_json(&payload.request_headers)?;
    let response_headers = parse_json(&payload.response_headers)?;
    let request_payload = decode_encoded(&payload.request_payload, false)?;
    let response_payload = decode_encoded(&payload.response_payload, true)?;
    let tag = parse_json(&payload.tag)?;
    let metadata = parse_json(&payload.metadata)?;
    Ok(json!({
        "path": payload.path,
        "requestHeaders": request_headers,
        "responseHeaders": response_headers,
        "method": payload.method,
        "requestPayload": request_payload,
        "responsePayload": response_payload,
        "ip": payload.ip,
        "destIp": payload.dest_ip,
        "time": if payload.time.chars().all(|character| character.is_ascii_digit()) {
            String::from("<millis>")
        } else {
            payload.time
        },
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

fn parse_json(encoded: &str) -> Result<Value, Box<HelperError>> {
    serde_json::from_str(encoded).map_err(|error| failure("JsonError", None, error.to_string()))
}

// the request payload is always double encoded; the empty response payload is a bare object
fn decode_encoded(encoded: &str, response_payload: bool) -> Result<Value, Box<HelperError>> {
    let outer = parse_json(encoded)?;
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
    parse_json(inner)
}

pub(super) fn send(input: &Input, fire_and_forget: bool) -> crate::helpers::HelperResult {
    let mut guardrail = super::settings(input)?;
    let mock = MockProvider::start(script(&input.answer)?)
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    guardrail.base_url.clone_from(&mock.base_url);
    let payload = guardrail
        .build_akto_payload(
            &super::akto_inputs(input),
            &input.request_data,
            input.status_code.unwrap_or(200),
            input.include_response,
        )
        .map_err(|error| failure("AktoError", None, error.to_string()))?;
    let answer = deliver(&guardrail, input, &payload, fire_and_forget)?;
    let calls = mock
        .finish()
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    report(&calls, answer)
}

fn deliver(
    guardrail: &AktoMirroring,
    input: &Input,
    payload: &AktoPayload,
    fire_and_forget: bool,
) -> Result<Option<Result<AktoAnswer, AktoError>>, Box<HelperError>> {
    let guardrails = input.guardrails.unwrap_or(false);
    let ingest_data = input.ingest_data.unwrap_or(false);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", None, error.to_string()))?;
    Ok(runtime.block_on(async {
        if fire_and_forget {
            guardrail
                .fire_and_forget_request(guardrails, ingest_data, payload)
                .await;
            None
        } else {
            Some(
                guardrail
                    .send_request(guardrails, ingest_data, payload)
                    .await,
            )
        }
    }))
}

fn report(
    calls: &[ProviderCall],
    answer: Option<Result<AktoAnswer, AktoError>>,
) -> crate::helpers::HelperResult {
    let request = calls
        .first()
        .ok_or_else(|| failure("MockError", None, "no request reached the mock".to_owned()))
        .and_then(|call| captured_request(&call.request))?;
    match answer {
        None => Ok(json!({"request": request, "succeeded": true})),
        Some(Ok(answer)) => {
            let verdict = AktoMirroring::handle_guardrail_response(&answer)
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
    MockScript::try_from(std::slice::from_ref(&reply))
        .map_err(|error| failure("ScriptError", None, error.to_string()))
}

fn captured_request(request: &ProviderRequest) -> Result<Value, Box<HelperError>> {
    let (path, query) = match request.path.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (request.path.as_str(), None),
    };
    let mut params = serde_json::Map::new();
    if let Some(query) = query {
        for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
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
