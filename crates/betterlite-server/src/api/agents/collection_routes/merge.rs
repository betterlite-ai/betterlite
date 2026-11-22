use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson as JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

pub const TEMPLATE_FIELD: &str = "litellm_params_template";
pub const PROVIDER_FIELD: &str = "custom_llm_provider";
pub const PROVIDER_DEFAULT: &str = "gemini";

// _safe_get_request_query_params reads Starlette's QueryParams as a dict: the last value of a repeated key wins
#[must_use]
pub fn query_values(raw_query: Option<&str>) -> BTreeMap<String, String> {
    form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}

// setdefault for every template entry: a key the route already fixed is never replaced
fn merge_template(data: &mut JsonMap, template: JsonMap) {
    for (key, value) in template {
        data.entry(key).or_insert(value);
    }
}

// json.loads(...) returning a dict; malformed JSON and every other decoded shape merge nothing
fn decoded_template(raw_template: &str) -> Option<JsonMap> {
    match serde_json::from_str::<JsonValue>(raw_template) {
        Ok(JsonValue::Object(template)) => Some(template),
        Ok(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_),
        )
        | Err(_) => None,
    }
}

// _merge_query_params_into_data: only the JSON-encoded template carries credentials, never a flat query key
pub fn merge_query_params_into_data(data: &mut JsonMap, query_params: &BTreeMap<String, String>) {
    if query_params.is_empty() {
        return;
    }
    let Some(raw_template) = query_params.get(TEMPLATE_FIELD) else {
        return;
    };
    if raw_template.is_empty() {
        return;
    }
    let Some(template) = decoded_template(raw_template) else {
        return;
    };
    merge_template(data, template);
}

// create's body template is popped first, and the body's own keys keep precedence over it
pub fn merge_body_template(data: &mut JsonMap) {
    if let Some(JsonValue::Object(template)) = data.remove(TEMPLATE_FIELD) {
        merge_template(data, template);
    }
}

// data.setdefault("custom_llm_provider", "gemini") runs after the merge, so a template may still choose the provider
pub fn default_provider(data: &mut JsonMap) {
    data.entry(String::from(PROVIDER_FIELD))
        .or_insert(JsonValue::String(String::from(PROVIDER_DEFAULT)));
}
