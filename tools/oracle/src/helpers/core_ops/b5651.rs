use betterlite_config::sections::settings::JsonValue;
use betterlite_core::responses_bridge::{
    BridgeRequest, ModelInfoError, ReasoningEffort, ResponsesBridgeCapabilities,
    responses_api_bridge_check,
};
use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    request: Request,
    capabilities: Facts,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    model: String,
    custom_llm_provider: String,
    #[serde(default)]
    web_search_options: Option<JsonValue>,
    #[serde(default)]
    tools: Vec<JsonValue>,
    #[serde(default)]
    reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    reasoning_summary: Option<JsonValue>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default, rename = "route_all_chat_openai_to_responses")]
    route_all_openai_to_responses: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(from = "bool")]
enum Toggle {
    #[default]
    Off,
    On,
}

impl From<bool> for Toggle {
    fn from(value: bool) -> Self {
        if value { Self::On } else { Self::Off }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Facts {
    #[serde(default, rename = "model_info")]
    known_model_info: Object,
    #[serde(default, rename = "model_info_fails")]
    model_info_fails: Toggle,
    #[serde(rename = "resolved_openai_api_base")]
    resolved_api_base: String,
    #[serde(default, rename = "foundry_supports_native_responses")]
    foundry_native_responses: Toggle,
    #[serde(default, rename = "foundry_chat_rejects_function_tools")]
    foundry_rejects_tools: Toggle,
    #[serde(default, rename = "is_gpt_5_model")]
    gpt_5: Toggle,
    #[serde(default, rename = "is_gpt_5_4_plus_model")]
    gpt_5_4_plus: Toggle,
    #[serde(default, rename = "is_gpt_5_search_model")]
    gpt_5_search: Toggle,
}

impl ResponsesBridgeCapabilities for Facts {
    fn model_info(
        &self,
        model: &str,
        _custom_llm_provider: &str,
    ) -> Result<Object, ModelInfoError> {
        if matches!(self.model_info_fails, Toggle::On) {
            return Err(ModelInfoError::Unknown {
                model: model.to_owned(),
            });
        }
        Ok(self.known_model_info.clone())
    }

    fn resolved_openai_api_base(&self, _api_base: Option<&str>) -> String {
        self.resolved_api_base.clone()
    }

    fn foundry_supports_responses(&self, _model: &str, _api_base: Option<&str>) -> bool {
        matches!(self.foundry_native_responses, Toggle::On)
    }

    fn foundry_chat_rejects_tools(
        &self,
        _model: &str,
        _reasoning_effort: Option<&ReasoningEffort>,
    ) -> bool {
        matches!(self.foundry_rejects_tools, Toggle::On)
    }

    fn is_gpt_5_model(&self, _model: &str) -> bool {
        matches!(self.gpt_5, Toggle::On)
    }

    fn is_gpt_5_4_plus_model(&self, _model: &str) -> bool {
        matches!(self.gpt_5_4_plus, Toggle::On)
    }

    fn is_gpt_5_search_model(&self, _model: &str) -> bool {
        matches!(self.gpt_5_search, Toggle::On)
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let fields = &input.request;
        let request = BridgeRequest {
            model: &fields.model,
            custom_llm_provider: &fields.custom_llm_provider,
            web_search_options: fields.web_search_options.as_ref(),
            tools: &fields.tools,
            reasoning_effort: fields.reasoning_effort.as_ref(),
            reasoning_summary: fields.reasoning_summary.as_ref(),
            api_base: fields.api_base.as_deref(),
            route_all_openai_to_responses: fields.route_all_openai_to_responses,
        };
        let check = responses_api_bridge_check(&request, &input.capabilities);
        Ok(json!({"model_info": check.model_info, "model": check.model}))
    }
}
