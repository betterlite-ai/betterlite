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
    let encoded = serde_json::to_string(&payload)
        .map_err(|error| failure("JsonError", None, error.to_string()))?;
    let raw: Value = serde_json::from_str(&encoded)
        .map_err(|error| failure("JsonError", None, error.to_string()))?;
    normalize(&raw)
}

pub(super) fn normalize(raw: &Value) -> crate::helpers::HelperResult {
    Ok(json!({
        "path": text_field(raw, "path")?,
        "requestHeaders": parse_json(&text_field(raw, "requestHeaders")?)?,
        "responseHeaders": parse_json(&text_field(raw, "responseHeaders")?)?,
        "method": text_field(raw, "method")?,
        "requestPayload": decode_double(&text_field(raw, "requestPayload")?)?,
        "responsePayload": decode_response_payload(&text_field(raw, "responsePayload")?)?,
        "ip": text_field(raw, "ip")?,
        "destIp": text_field(raw, "destIp")?,
        "time": clock_text(&text_field(raw, "time")?),
        "statusCode": text_field(raw, "statusCode")?,
        "type": text_field(raw, "type")?,
        "status": text_field(raw, "status")?,
        "akto_account_id": text_field(raw, "akto_account_id")?,
        "akto_vxlan_id": text_field(raw, "akto_vxlan_id")?,
        "is_pending": text_field(raw, "is_pending")?,
        "source": text_field(raw, "source")?,
        "direction": null_field(raw, "direction")?,
        "process_id": null_field(raw, "process_id")?,
        "socket_id": null_field(raw, "socket_id")?,
        "daemonset_id": null_field(raw, "daemonset_id")?,
        "enabled_graph": null_field(raw, "enabled_graph")?,
        "tag": parse_json(&text_field(raw, "tag")?)?,
        "metadata": parse_json(&text_field(raw, "metadata")?)?,
        "contextSource": text_field(raw, "contextSource")?,
    }))
}

fn field<'value>(value: &'value Value, name: &str) -> Result<&'value Value, Box<HelperError>> {
    value.get(name).ok_or_else(|| {
        failure(
            "AktoError",
            None,
            format!("payload field {name} is missing"),
        )
    })
}

fn text_field(value: &Value, name: &str) -> Result<String, Box<HelperError>> {
    match field(value, name)? {
        Value::String(text) => Ok(text.clone()),
        other => Err(failure(
            "AktoError",
            None,
            format!("payload field {name} is not text: {other}"),
        )),
    }
}

fn null_field(value: &Value, name: &str) -> Result<Value, Box<HelperError>> {
    let found = field(value, name)?;
    if found.is_null() {
        return Ok(Value::Null);
    }
    Err(failure(
        "AktoError",
        None,
        format!("payload field {name} is not null: {found}"),
    ))
}

fn parse_json(encoded: &str) -> Result<Value, Box<HelperError>> {
    serde_json::from_str(encoded).map_err(|error| failure("JsonError", None, error.to_string()))
}

fn decode_double(encoded: &str) -> Result<Value, Box<HelperError>> {
    let outer = parse_json(encoded)?;
    let inner = text_field(&outer, "body")?;
    parse_json(&inner)
}

fn decode_response_payload(encoded: &str) -> Result<Value, Box<HelperError>> {
    let outer = parse_json(encoded)?;
    if outer.get("body").is_none() {
        return Ok(outer);
    }
    decode_double(encoded)
}

fn clock_text(text: &str) -> String {
    if text.chars().all(|character| character.is_ascii_digit()) {
        return "<millis>".to_owned();
    }
    text.to_owned()
}
