use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// the row shape _scoped_spend_report_sql returns for a key, user or team scope
#[derive(Clone, Deserialize, Serialize)]
pub struct ScopedKeySpend {
    pub api_key: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_cost: Decimal,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub model_details: Vec<ModelSpend>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelSpend {
    pub model: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_cost: Decimal,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
}

// the same row shape from _ORG_SPEND_REPORT_SQL, whose model details also carry the team
#[derive(Clone, Deserialize, Serialize)]
pub struct OrganizationKeySpend {
    pub api_key: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_cost: Decimal,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub model_details: Vec<OrganizationModelSpend>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrganizationModelSpend {
    pub team_id: Option<String>,
    pub model: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_cost: Decimal,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
}

// get_spend_by_team: one entry per day, each with its teams and their per-model, per-key metadata
#[derive(Clone, Deserialize, Serialize)]
pub struct DailyTeamSpend {
    pub group_by_day: String,
    pub teams: Vec<TeamSpend>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TeamSpend {
    pub team_name: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_spend: Decimal,
    pub metadata: Vec<TeamModelSpend>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TeamModelSpend {
    pub model: String,
    pub api_key: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    pub total_tokens: i64,
}
