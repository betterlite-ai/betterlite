use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Ai21Params,
    Ai21Map {
        params: Object,
        #[serde(default)]
        optional_params: Object,
    },
    MinimaxParams {
        model: String,
        supports_reasoning: bool,
    },
    MinimaxInfo {
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        env_base: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        env_key: Option<String>,
        #[serde(default)]
        global_key: Option<String>,
    },
    MinimaxUrl {
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        env_base: Option<String>,
    },
    ModelscopeUrl {
        #[serde(default)]
        api_base: Option<String>,
    },
    TencentParams {
        model: String,
        supports_reasoning: bool,
    },
    TencentUrl {
        #[serde(default)]
        api_base: Option<String>,
    },
    TencentMap {
        model: String,
        supports_reasoning: bool,
        #[serde(default)]
        supports_adaptive_thinking: bool,
        params: Object,
        #[serde(default)]
        optional_params: Object,
    },
    TencentCost {
        model: String,
        prompt_tokens: i64,
        completion_tokens: i64,
    },
}
