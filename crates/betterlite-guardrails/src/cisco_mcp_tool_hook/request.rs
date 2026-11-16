use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_logging::mcp_tool_hook::types::McpPostCallResponse;
use betterlite_models::decisions::Json as DecisionsJson;
use indexmap::IndexMap;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::blocking::extract_tool_call_response;
use crate::cisco_verdict_enforcement::support::insert_serialized;

const COPIED_KEYS: [&str; 14] = [
    "name",
    "litellm_call_id",
    "id",
    "user",
    "mcp_tool_name",
    "tool_name",
    "mcp_arguments",
    "arguments",
    "mcp_server_name",
    "server_name",
    "metadata",
    "litellm_metadata",
    "mcp_tool_call_metadata",
    "guardrails",
];

pub(super) fn request_fields(kwargs: &IndexMap<String, DecisionsJson>) -> Fields {
    let mut data = Fields::new();
    for key in COPIED_KEYS {
        if let Some(value) = kwargs.get(key)
            && !matches!(value, DecisionsJson::Null)
        {
            insert_serialized(&mut data, key, value);
        }
    }
    data
}

pub(super) fn guardrail_data(request_data: &Fields) -> BTreeMap<String, GuardrailValue> {
    let mut data = BTreeMap::new();
    for (key, raw) in request_data {
        data.insert(key.clone(), guardrail_value(&Json::parse(raw.get())));
    }
    data
}

fn guardrail_value(value: &Json) -> GuardrailValue {
    match value {
        Json::Null => GuardrailValue::Null,
        Json::Bool(flag) => GuardrailValue::Boolean(*flag),
        Json::Number(text) => number_value(text),
        Json::Text(text) => GuardrailValue::Text(text.clone()),
        Json::List(items) => GuardrailValue::Array(
            items
                .iter()
                .map(|item| guardrail_value(&Json::parse(item.get())))
                .collect(),
        ),
        Json::Object(fields) => GuardrailValue::Object(
            fields
                .iter()
                .map(|(key, raw)| (key.clone(), guardrail_value(&Json::parse(raw.get()))))
                .collect(),
        ),
    }
}

fn number_value(text: &str) -> GuardrailValue {
    if let Ok(signed) = text.parse::<i64>() {
        return GuardrailValue::SignedInteger(signed);
    }
    if let Ok(unsigned) = text.parse::<u64>() {
        return GuardrailValue::UnsignedInteger(unsigned);
    }
    match text.parse::<f64>() {
        Ok(decimal) => GuardrailValue::Decimal(decimal),
        Err(_) => GuardrailValue::Text(text.to_owned()),
    }
}

pub(super) fn scanned_response(response: &McpPostCallResponse) -> Option<Json> {
    let mut wrapper = Fields::new();
    insert_serialized(
        &mut wrapper,
        "mcp_tool_call_response",
        &response.mcp_tool_call_response,
    );
    let extracted = extract_tool_call_response(&Json::Object(wrapper));
    if matches!(extracted, Json::Null) {
        return None;
    }
    Some(extracted)
}

pub(super) fn original_present(kwargs: &IndexMap<String, DecisionsJson>) -> bool {
    kwargs
        .get("original_response")
        .is_some_and(|value| !matches!(value, DecisionsJson::Null))
}
