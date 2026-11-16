use betterlite_models::decisions::Json as JsonValue;
use betterlite_models::prompt_spec::{PromptInfo, PromptSpec};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::registry::prompt_registry_helpers::get_base_prompt_id;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::Serialize;

use crate::cache_settings::REDACTED_VALUE;

#[derive(Serialize)]
pub(super) struct ListPromptsResponse {
    pub(super) prompts: Vec<PromptSpecWire>,
}

#[derive(Serialize)]
pub(super) struct PromptInfoResponse {
    pub(super) prompt_spec: PromptSpecWire,
    pub(super) raw_prompt_template: Option<PromptTemplateBase>,
    pub(super) environments: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct PromptTemplateBase {
    pub(super) litellm_prompt_id: String,
    pub(super) content: String,
    pub(super) metadata: Option<Object>,
}

#[derive(Serialize)]
pub(super) struct PromptSpecWire {
    prompt_id: String,
    litellm_params: PromptLiteLLMParamsWire,
    prompt_info: PromptInfo,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    version: Option<i64>,
    environment: String,
    created_by: Option<String>,
}

#[derive(Serialize)]
struct PromptLiteLLMParamsWire {
    prompt_id: Option<String>,
    prompt_integration: String,
    api_base: Option<String>,
    api_key: Option<String>,
    provider_specific_query_params: Option<IndexMap<String, JsonValue>>,
    ignore_prompt_manager_model: bool,
    #[serde(rename = "ignore_prompt_manager_optional_params")]
    ignore_optional_params: bool,
    dotprompt_content: Option<String>,
    #[serde(flatten)]
    extra: IndexMap<String, JsonValue>,
}

impl From<&PromptSpec> for PromptSpecWire {
    fn from(spec: &PromptSpec) -> Self {
        let params = &spec.litellm_params;
        Self {
            prompt_id: spec.prompt_id.clone(),
            litellm_params: PromptLiteLLMParamsWire {
                prompt_id: params.prompt_id.clone(),
                prompt_integration: params.prompt_integration.clone(),
                api_base: params.api_base.clone(),
                // LiteLLM repeats a configured prompt key in plaintext; betterlite never sends a secret back
                api_key: params
                    .api_key
                    .as_ref()
                    .map(|_secret| REDACTED_VALUE.to_owned()),
                provider_specific_query_params: params.provider_specific_query_params.clone(),
                ignore_prompt_manager_model: params.ignore_model,
                ignore_optional_params: params.ignore_optional_params,
                dotprompt_content: params.dotprompt_content.clone(),
                extra: params.extra.clone(),
            },
            prompt_info: spec.prompt_info.clone(),
            created_at: spec.created_at,
            updated_at: spec.updated_at,
            version: spec.version,
            environment: spec.environment.clone(),
            created_by: spec.created_by.clone(),
        }
    }
}

// LiteLLM answers display copies: the base prompt id, and only the version a history row names
pub(super) fn display_wire(spec: &PromptSpec, version: Option<i64>) -> PromptSpecWire {
    let mut wire = PromptSpecWire::from(spec);
    wire.prompt_id = get_base_prompt_id(&wire.prompt_id);
    wire.version = version;
    wire
}
