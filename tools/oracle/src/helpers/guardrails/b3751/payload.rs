use betterlite_guardrails::akto::payload::AktoPayload;
use serde_json::{Value, json};

use super::Input;
use super::support::failure;
use crate::helpers::HelperError;

pub(super) fn build(input: &Input) -> crate::helpers::HelperResult {
    let guardrail = super::settings::build(input)?;
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
        "time": clock_text(&payload.time),
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

fn clock_text(text: &str) -> String {
    if text.chars().all(|character| character.is_ascii_digit()) {
        return "<millis>".to_owned();
    }
    text.to_owned()
}
