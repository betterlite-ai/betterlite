use serde_json::Value;

use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

const RESPONSE_FORMAT: &str = "response_format";

// LlamaAPIConfig redefines only the mapping; the supported list is OpenAIGPTConfig's
#[derive(Debug, Clone, Copy, Default)]
pub struct LlamaAPIConfig;

impl LlamaAPIConfig {
    // the parent's mapping, then response_format goes unless its type is json_schema
    pub fn map_openai_params(
        non_default_params: &Object,
        optional_params: &mut Object,
        model: &str,
        catalog: &CatalogCapabilities,
    ) {
        OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        }
        .map_params(model, non_default_params, optional_params, catalog);
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
