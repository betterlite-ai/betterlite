use std::collections::BTreeMap;

use betterlite_bedrock_guardrails::bedrock_transport::BedrockGuardrail;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde_json::Number;

use crate::bedrock_records::ChecksViolation;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::{GuardrailFailure, ModifyResponseFailure};

const BLOCK_MESSAGE: &str = "Violated guardrail policy";
const DEFAULT_MODEL: &str = "bedrock-guardrail";

// LiteLLM's _get_block_exception_for_checks
#[must_use]
pub fn exception(
    guardrail: &BedrockGuardrail,
    base: &GuardrailBase,
    violations: &[ChecksViolation],
    request_data: &BTreeMap<String, GuardrailValue>,
) -> GuardrailFailure {
    if guardrail.disable_exception_on_block {
        return GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
            message: BLOCK_MESSAGE.to_owned(),
            model: blocked_model(request_data),
            request_data: request_data.clone(),
            guardrail_name: base.settings.guardrail_name.clone(),
            detection_info: BTreeMap::new(),
            original_response: None,
        }));
    }
    let mut detail = IndexMap::new();
    detail.insert(
        "error".to_owned(),
        GuardrailValue::Text(BLOCK_MESSAGE.to_owned()),
    );
    detail.insert(
        "bedrock_guardrail_checks".to_owned(),
        GuardrailValue::Array(violations.iter().map(violation_value).collect()),
    );
    GuardrailFailure::HttpObject {
        status: 400,
        detail,
    }
}

fn blocked_model(request_data: &BTreeMap<String, GuardrailValue>) -> String {
    match request_data.get("model") {
        Some(GuardrailValue::Text(model)) => model.clone(),
        _ => DEFAULT_MODEL.to_owned(),
    }
}

// LiteLLM hands the caller's violation dict back, so a missing label is written as an explicit null
fn violation_value(violation: &ChecksViolation) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    insert_text(&mut fields, "check", violation.check.as_deref());
    if let Some(score) = &violation.severity_score {
        insert_text(&mut fields, "category", violation.category.as_deref());
        insert_number(&mut fields, "severityScore", score);
    } else if let Some(score) = &violation.confidence_score {
        insert_text(&mut fields, "type", violation.type_name.as_deref());
        insert_number(&mut fields, "confidenceScore", score);
    }
    if violation.truncated == Some(true) {
        fields.insert("truncated".to_owned(), GuardrailValue::Boolean(true));
    }
    GuardrailValue::Object(fields)
}

fn insert_text(fields: &mut BTreeMap<String, GuardrailValue>, key: &str, value: Option<&str>) {
    let encoded = match value {
        Some(text) => GuardrailValue::Text(text.to_owned()),
        _ => GuardrailValue::Null,
    };
    fields.insert(key.to_owned(), encoded);
}

fn insert_number(fields: &mut BTreeMap<String, GuardrailValue>, key: &str, number: &Number) {
    let encoded = match number.as_f64() {
        Some(value) => GuardrailValue::Decimal(value),
        _ => GuardrailValue::Null,
    };
    fields.insert(key.to_owned(), encoded);
}
