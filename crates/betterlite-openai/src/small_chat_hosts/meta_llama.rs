use serde_json::Value;

use crate::openai_base_config::{CatalogCapabilities, Object};

const RESPONSE_FORMAT: &str = "response_format";

// LlamaAPIConfig redefines only these two methods; the rest is OpenAIGPTConfig
#[derive(Debug, Clone, Copy, Default)]
pub struct LlamaAPIConfig;

impl LlamaAPIConfig {
    // the OpenAI list, unchanged
    #[must_use]
    pub fn supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
        super::parent_supported_params(model, catalog)
    }

    // the parent's mapping, then response_format goes unless its type is json_schema
    pub fn map_openai_params(
        non_default_params: &Object,
        optional_params: &mut Object,
        model: &str,
        catalog: &CatalogCapabilities,
    ) {
        super::openai_parent().map_params(model, non_default_params, optional_params, catalog);
        if response_format_is_json_schema(optional_params) {
            return;
        }
        optional_params.remove(RESPONSE_FORMAT);
    }
}

// a response_format with no type, or any other type, is dropped
fn response_format_is_json_schema(optional_params: &Object) -> bool {
    optional_params
        .get(RESPONSE_FORMAT)
        .and_then(Value::as_object)
        .and_then(|fields| fields.get("type"))
        .is_some_and(|kind| kind.as_str() == Some("json_schema"))
}
