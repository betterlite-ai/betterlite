use std::collections::BTreeMap;
use std::iter::once;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::response_tool_calls::tools::has_tool_with_name;

use crate::generic_guardrail_contracts::GuardrailToolParam;
use crate::headroom_settings::HEADROOM_RETRIEVE_TOOL_NAME;

const FUNCTION_KEY: &str = "function";
const TOOL_DESCRIPTION: &str = "Retrieve original content that was compressed by Headroom. Call this when you encounter a compression marker containing a hash.";
const HASH_DESCRIPTION: &str = "The hex hash from the compression marker.";
const QUERY_DESCRIPTION: &str = "Optional search query for BM25-ranked retrieval.";
const HASH_PARAMETER: &str = "hash";
const QUERY_PARAMETER: &str = "query";

pub(crate) fn build() -> GuardrailToolParam {
    GuardrailToolParam {
        tool_type: FUNCTION_KEY.to_owned(),
        extra: BTreeMap::from([(FUNCTION_KEY.to_owned(), function_schema())]),
    }
}

fn function_schema() -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "description".to_owned(),
            GuardrailValue::Text(TOOL_DESCRIPTION.to_owned()),
        ),
        (
            "name".to_owned(),
            GuardrailValue::Text(HEADROOM_RETRIEVE_TOOL_NAME.to_owned()),
        ),
        ("parameters".to_owned(), parameters_schema()),
    ]))
}

fn parameters_schema() -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "properties".to_owned(),
            GuardrailValue::Object(BTreeMap::from([
                (HASH_PARAMETER.to_owned(), string_schema(HASH_DESCRIPTION)),
                (QUERY_PARAMETER.to_owned(), string_schema(QUERY_DESCRIPTION)),
            ])),
        ),
        (
            "required".to_owned(),
            GuardrailValue::Array(vec![GuardrailValue::Text(HASH_PARAMETER.to_owned())]),
        ),
        ("type".to_owned(), GuardrailValue::Text("object".to_owned())),
    ]))
}

fn string_schema(description: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "description".to_owned(),
            GuardrailValue::Text(description.to_owned()),
        ),
        ("type".to_owned(), GuardrailValue::Text("string".to_owned())),
    ]))
}

pub(crate) fn has_retrieve_tool(tools: &[GuardrailToolParam]) -> bool {
    let Ok(encoded) = serde_json::to_value(tools) else {
        return false;
    };
    let Ok(rows) = serde_json::from_value::<Vec<Json>>(encoded) else {
        return false;
    };
    has_tool_with_name(&rows, HEADROOM_RETRIEVE_TOOL_NAME)
}

pub(crate) fn merged(existing: Option<&[GuardrailToolParam]>) -> Vec<GuardrailToolParam> {
    match existing {
        None => vec![build()],
        Some(tools) if has_retrieve_tool(tools) => tools.to_vec(),
        Some(tools) => tools.iter().cloned().chain(once(build())).collect(),
    }
}
