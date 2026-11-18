use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::TeamBase;
use serde::{Deserialize, Serialize};

use crate::key_fields::values::{AllowedVectorStoreIndexItem, Amount, LiteLLMObjectPermissionBase};

use super::TeamContractError;
use super::shared::{
    RequestProjection, blank_team_id_to_null, decode_request, normalize_budget_aliases, object,
    validate_router_settings,
};
use super::values::{BudgetConfig, PositiveInt, ThroughputLimitType};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewTeamFields {
    pub model_aliases: Option<BTreeMap<String, JsonData>>,
    pub model_max_budget: Option<BTreeMap<String, BudgetConfig>>,
    pub tags: Option<Vec<JsonData>>,
    pub guardrails: Option<Vec<String>>,
    pub policies: Option<Vec<String>>,
    pub prompts: Option<Vec<String>>,
    pub object_permission: Option<LiteLLMObjectPermissionBase>,
    pub allowed_passthrough_routes: Option<Vec<JsonData>>,
    pub disable_global_guardrails: Option<bool>,
    pub secret_manager_settings: Option<BTreeMap<String, JsonData>>,
    pub model_rpm_limit: Option<BTreeMap<String, i64>>,
    pub rpm_limit_type: Option<ThroughputLimitType>,
    pub tpm_limit_type: Option<ThroughputLimitType>,
    pub model_tpm_limit: Option<BTreeMap<String, i64>>,
    #[serde(rename = "default_estimated_output_tokens")]
    pub estimated_output_tokens: Option<PositiveInt>,
    #[serde(rename = "default_estimated_output_tokens_per_model")]
    pub per_model_output_tokens: Option<BTreeMap<String, PositiveInt>>,
    pub mcp_rpm_limit: Option<BTreeMap<String, i64>>,
    pub team_member_budget: Option<Amount>,
    pub team_member_rpm_limit: Option<i64>,
    pub team_member_tpm_limit: Option<i64>,
    pub team_member_key_duration: Option<String>,
    pub team_member_budget_duration: Option<String>,
    pub allowed_vector_store_indexes: Option<Vec<AllowedVectorStoreIndexItem>>,
    #[serde(rename = "enforced_batch_output_expires_after")]
    pub enforced_batch_expiry: Option<BTreeMap<String, JsonData>>,
    pub enforced_file_expires_after: Option<BTreeMap<String, JsonData>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewTeamRequest {
    #[serde(flatten)]
    pub team: TeamBase<Option<String>>,
    #[serde(flatten)]
    pub extra: NewTeamFields,
    #[serde(skip)]
    pub(crate) raw: JsonData,
}

impl NewTeamRequest {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let mut normalized = object(value)?.clone();
        blank_team_id_to_null(&mut normalized);
        normalize_budget_aliases(&mut normalized);
        let data = JsonData::Object(normalized);
        let team: TeamBase<Option<String>> = decode_request(&data)?;
        let extra: NewTeamFields = decode_request(&data)?;
        validate_router_settings(team.router_settings.as_ref())?;
        Ok(Self {
            team,
            extra,
            raw: data,
        })
    }
}

impl RequestProjection for NewTeamRequest {
    fn supplied_document(&self) -> &JsonData {
        &self.raw
    }

    fn supplied_permission(&self) -> Option<&LiteLLMObjectPermissionBase> {
        self.extra.object_permission.as_ref()
    }
}
