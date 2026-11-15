#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::prompt_spec::{PromptInfo, PromptLiteLLMParams, PromptSpec};
use indexmap::IndexMap;

pub(crate) fn prompt_params() -> PromptLiteLLMParams {
    PromptLiteLLMParams {
        prompt_id: None,
        prompt_integration: "dotprompt".to_owned(),
        api_base: None,
        api_key: None,
        provider_specific_query_params: None,
        ignore_model: false,
        ignore_optional_params: false,
        dotprompt_content: None,
        extra: IndexMap::new(),
    }
}

pub(crate) fn prompt_spec(prompt_id: &str, version: Option<i64>, environment: &str) -> PromptSpec {
    PromptSpec {
        prompt_id: prompt_id.to_owned(),
        litellm_params: prompt_params(),
        prompt_info: PromptInfo::default(),
        created_at: None,
        updated_at: None,
        version,
        environment: environment.to_owned(),
        created_by: None,
    }
}

pub(crate) fn json_value(text: &str) -> Json {
    serde_json::from_str(text).expect("test JSON parses")
}
