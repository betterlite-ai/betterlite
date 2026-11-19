use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::member::BudgetLimitEntry;
use serde::{Deserialize, Serialize};

use crate::key_fields::values::{AllowedVectorStoreIndexItem, Amount, LiteLLMObjectPermissionBase};

use super::TeamContractError;
use super::shared::{
    RequestProjection, decode_request, normalize_budget_aliases, object, validate_router_settings,
};
use super::values::{BudgetConfig, PositiveInt};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateTeamFields {
    pub team_alias: Option<String>,
    pub organization_id: Option<String>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub tpd_limit: Option<i64>,
    pub max_budget: Option<Amount>,
    pub soft_budget: Option<Amount>,
    pub models: Option<Vec<JsonData>>,
    pub blocked: Option<bool>,
    pub budget_duration: Option<String>,
    pub tags: Option<Vec<JsonData>>,
    pub model_aliases: Option<BTreeMap<String, JsonData>>,
    pub guardrails: Option<Vec<String>>,
    pub policies: Option<Vec<String>>,
    pub object_permission: Option<LiteLLMObjectPermissionBase>,
    pub disable_global_guardrails: Option<bool>,
    pub team_member_budget: Option<Amount>,
    pub team_member_budget_duration: Option<String>,
    pub team_member_rpm_limit: Option<i64>,
    pub team_member_tpm_limit: Option<i64>,
    pub team_member_key_duration: Option<String>,
    pub allowed_passthrough_routes: Option<Vec<JsonData>>,
    pub secret_manager_settings: Option<BTreeMap<String, JsonData>>,
    pub prompts: Option<Vec<String>>,
    pub model_rpm_limit: Option<BTreeMap<String, i64>>,
    pub model_tpm_limit: Option<BTreeMap<String, i64>>,
    #[serde(rename = "default_estimated_output_tokens")]
    pub estimated_output_tokens: Option<PositiveInt>,
    #[serde(rename = "default_estimated_output_tokens_per_model")]
    pub per_model_output_tokens: Option<BTreeMap<String, PositiveInt>>,
    pub mcp_rpm_limit: Option<BTreeMap<String, i64>>,
    pub allowed_vector_store_indexes: Option<Vec<AllowedVectorStoreIndexItem>>,
    #[serde(rename = "enforced_batch_output_expires_after")]
    pub enforced_batch_expiry: Option<BTreeMap<String, JsonData>>,
    pub enforced_file_expires_after: Option<BTreeMap<String, JsonData>>,
    pub router_settings: Option<BTreeMap<String, JsonData>>,
    pub access_group_ids: Option<Vec<String>>,
    pub budget_limits: Option<Vec<BudgetLimitEntry>>,
    pub default_team_member_models: Option<Vec<String>>,
    pub model_max_budget: Option<BTreeMap<String, BudgetConfig>>,
}

#[derive(Deserialize)]
struct OptionalTeamIdInput {
    team_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateTeamRequest {
    pub team_id: String,
    #[serde(flatten)]
    pub fields: UpdateTeamFields,
    #[serde(skip)]
    pub raw: JsonData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PatchTeamRequest {
    pub team_id: Option<String>,
    #[serde(flatten)]
    pub fields: UpdateTeamFields,
    #[serde(skip)]
    pub raw: JsonData,
}

impl PatchTeamRequest {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let data = normalized(value)?;
        let team_id: OptionalTeamIdInput = decode_request(&data)?;
        let fields: UpdateTeamFields = decode_request(&data)?;
        validate_router_settings(fields.router_settings.as_ref())?;
        Ok(Self {
            team_id: team_id.team_id,
            fields,
            raw: data,
        })
    }
}

impl UpdateTeamRequest {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let patch = PatchTeamRequest::from_value(value)?;
        match patch.team_id {
            Some(team_id) => Ok(Self {
                team_id,
                fields: patch.fields,
                raw: patch.raw,
            }),
            None => Err(TeamContractError::Validation {
                message: "team_id: Input should be a valid string".to_owned(),
            }),
        }
    }
}

impl RequestProjection for UpdateTeamRequest {
    fn supplied_document(&self) -> &JsonData {
        &self.raw
    }

    fn supplied_permission(&self) -> Option<&LiteLLMObjectPermissionBase> {
        self.fields.object_permission.as_ref()
    }
}

impl RequestProjection for PatchTeamRequest {
    fn supplied_document(&self) -> &JsonData {
        &self.raw
    }

    fn supplied_permission(&self) -> Option<&LiteLLMObjectPermissionBase> {
        self.fields.object_permission.as_ref()
    }
}

fn normalized(value: &JsonData) -> Result<JsonData, TeamContractError> {
    let mut fields = object(value)?.clone();
    normalize_budget_aliases(&mut fields);
    Ok(JsonData::Object(fields))
}
