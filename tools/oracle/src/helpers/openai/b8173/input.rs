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
    Ai21Map(Ai21MapStep),
    MinimaxParams(MinimaxParamsStep),
    MinimaxInfo(MinimaxInfoStep),
    MinimaxUrl(MinimaxUrlStep),
    ModelscopeUrl(ModelscopeUrlStep),
    TencentParams(TencentParamsStep),
    TencentUrl(TencentUrlStep),
    TencentMap(TencentMapStep),
    TencentCost(TencentCostStep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ai21MapStep {
    pub(crate) params: Object,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MinimaxParamsStep {
    pub(crate) model: String,
    pub(crate) supports_reasoning: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MinimaxInfoStep {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) env_base: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) env_key: Option<String>,
    #[serde(default)]
    pub(crate) global_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MinimaxUrlStep {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) env_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelscopeUrlStep {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TencentParamsStep {
    pub(crate) model: String,
    pub(crate) supports_reasoning: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TencentUrlStep {
    #[serde(default)]
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TencentMapStep {
    pub(crate) model: String,
    pub(crate) supports_reasoning: bool,
    #[serde(default)]
    pub(crate) supports_adaptive_thinking: bool,
    pub(crate) params: Object,
    #[serde(default)]
    pub(crate) optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TencentCostStep {
    pub(crate) model: String,
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
}
