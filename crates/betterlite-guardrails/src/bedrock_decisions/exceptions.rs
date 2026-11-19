use std::collections::BTreeMap;
use std::str;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::assessments::{
    VIOLATED_POLICY, blocked_assessments, redact_assessments, should_raise_blocked,
};
use super::value::json_to_value;
use crate::bedrock_records::Response;
use crate::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};
use crate::guardrail_records::record::GuardrailStatus;

const DEFAULT_MODEL: &str = "bedrock-guardrail";

// LiteLLM's _check_bedrock_response_for_exception reads the marker only inside a string Output.__type
#[must_use]
pub fn payload_has_exception(payload: &Json) -> bool {
    let Json::Object(fields) = payload else {
        return false;
    };
    let Some(Json::Object(output)) = fields.get("Output") else {
        return false;
    };
    let Some(Json::Text(kind)) = output.get("__type") else {
        return false;
    };
    kind.contains("Exception")
}

// the transport already parsed the body, so json(), content and text are one decoder here
#[must_use]
pub fn body_has_exception(body: &[u8]) -> bool {
    str::from_utf8(body).is_ok_and(|text| {
        serde_json::from_str::<Json>(text).is_ok_and(|payload| payload_has_exception(&payload))
    })
}

#[must_use]
pub fn response_has_exception(response: &Response) -> bool {
    let Some(raw) = response.extra.get("Output") else {
        return false;
    };
    serde_json::from_str::<Json>(raw.get()).is_ok_and(|payload| payload_has_exception(&payload))
}

// LiteLLM's _get_bedrock_guardrail_response_status: only a 200 can be a success or a block
#[must_use]
pub fn response_status(status_code: u16, body: &[u8]) -> GuardrailStatus {
    if status_code != 200 {
        return GuardrailStatus::GuardrailFailedToRespond;
    }
    if body_has_exception(body) {
        return GuardrailStatus::GuardrailFailedToRespond;
    }
    if let Ok(text) = str::from_utf8(body)
        && let Ok(response) = serde_json::from_str::<Response>(text)
        && should_raise_blocked(&response)
    {
        return GuardrailStatus::GuardrailIntervened;
    }
    GuardrailStatus::Success
}

// the merged response keeps an AWS exception marker that the chunk merge carried along
#[must_use]
pub fn merged_response_status(response: &Response) -> GuardrailStatus {
    if response_has_exception(response) {
        return GuardrailStatus::GuardrailFailedToRespond;
    }
    GuardrailStatus::Success
}

pub struct BlockedGuardrail<'name> {
    pub identifier: Option<&'name str>,
    pub version: Option<&'name str>,
    pub name: Option<&'name str>,
    pub disable_exception_on_block: bool,
}

// LiteLLM's _get_http_exception_for_blocked_guardrail
#[must_use]
pub fn http_exception_for_blocked_guardrail(
    response: &Response,
    request_data: &BTreeMap<String, GuardrailValue>,
    guardrail: &BlockedGuardrail<'_>,
) -> GuardrailFailure {
    let text = output_text(response);
    if guardrail.disable_exception_on_block {
        return modify_failure(text, request_data, guardrail);
    }
    GuardrailFailure::HttpObject {
        status: 400,
        detail: http_detail(response, text, guardrail),
    }
}

fn modify_failure(
    text: String,
    request_data: &BTreeMap<String, GuardrailValue>,
    guardrail: &BlockedGuardrail<'_>,
) -> GuardrailFailure {
    let mut model = DEFAULT_MODEL.to_owned();
    if let Some(GuardrailValue::Text(found)) = request_data.get("model") {
        model = found.clone();
    }
    GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
        message: text,
        model,
        request_data: request_data.clone(),
        guardrail_name: guardrail.name.map(str::to_owned),
        detection_info: BTreeMap::new(),
        original_response: None,
    }))
}

fn http_detail(
    response: &Response,
    text: String,
    guardrail: &BlockedGuardrail<'_>,
) -> IndexMap<String, GuardrailValue> {
    let mut detail = IndexMap::from([
        (
            "error".to_owned(),
            GuardrailValue::Text(VIOLATED_POLICY.to_owned()),
        ),
        (
            "bedrock_guardrail_response".to_owned(),
            GuardrailValue::Text(text),
        ),
    ]);
    if let Some(identifier) = guardrail.identifier.filter(|value| !value.is_empty()) {
        detail.insert(
            "guardrailIdentifier".to_owned(),
            GuardrailValue::Text(identifier.to_owned()),
        );
    }
    if let Some(version) = guardrail.version.filter(|value| !value.is_empty()) {
        detail.insert(
            "guardrailVersion".to_owned(),
            GuardrailValue::Text(version.to_owned()),
        );
    }
    let assessments = blocked_assessments(response);
    if !assessments.is_empty() {
        detail.insert(
            "assessments".to_owned(),
            json_to_value(&Json::List(redact_assessments(&assessments))),
        );
    }
    detail
}

fn output_text(response: &Response) -> String {
    let mut text = String::new();
    for output in response.outputs.as_deref().unwrap_or_default() {
        if let Some(chunk) = output.text.as_deref().filter(|chunk| !chunk.is_empty()) {
            text.push_str(chunk);
        }
    }
    text
}
