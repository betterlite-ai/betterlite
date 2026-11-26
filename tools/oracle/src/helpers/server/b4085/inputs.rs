use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Pipeline(Box<PipelineCall>),
}

#[derive(Deserialize)]
pub(crate) struct PipelineCall {
    #[serde(default)]
    pub(crate) auth: Option<AuthInput>,
    #[serde(default)]
    pub(crate) litellm_settings: LiteLLMSettingsInput,
    #[serde(default)]
    pub(crate) key_spend: Option<Decimal>,
    #[serde(default)]
    pub(crate) team: Option<TeamInput>,
    #[serde(default)]
    pub(crate) team_spend: Option<Decimal>,
    pub(crate) database: bool,
    pub(crate) pipeline_error: bool,
    #[serde(default)]
    pub(crate) max_budget: Option<Decimal>,
    #[serde(default)]
    pub(crate) job_spend: Option<Decimal>,
    #[serde(default)]
    pub(crate) stats_spend: Option<Decimal>,
    pub(crate) judge_reply: String,
    pub(crate) arms: Vec<ArmInput>,
}

#[derive(Deserialize)]
pub(crate) struct AuthInput {
    pub(crate) token: String,
    pub(crate) spend: Decimal,
    #[serde(default)]
    pub(crate) max_budget: Option<Decimal>,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
    #[serde(default)]
    pub(crate) metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) tpm_limit: Option<i64>,
    #[serde(default)]
    pub(crate) rpm_limit: Option<i64>,
}

#[derive(Default, Deserialize)]
pub(crate) struct LiteLLMSettingsInput {
    #[serde(default, rename = "budget_exceeded_throttle_percentage")]
    pub(crate) throttle_pct: Option<f64>,
}

#[derive(Deserialize)]
pub(crate) struct TeamInput {
    pub(crate) team_id: String,
    pub(crate) max_budget: Decimal,
    pub(crate) spend: Decimal,
}

#[derive(Deserialize)]
pub(crate) struct ArmInput {
    pub(crate) router: String,
    pub(crate) shadow: String,
}
