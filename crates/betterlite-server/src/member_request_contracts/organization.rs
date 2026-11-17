use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::{
    JsonData, LiteLLMBudgetTable, OrganizationMembership, Timestamp,
};
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::key_fields::values::LiteLLMObjectPermissionBase;

use super::{MemberContractError, field, json_object};

pub const MANAGEMENT_METADATA_FIELDS: [&str; 19] = [
    "model_rpm_limit",
    "model_tpm_limit",
    "model_itpm_limit",
    "model_otpm_limit",
    "default_estimated_output_tokens",
    "default_estimated_output_tokens_per_model",
    "mcp_rpm_limit",
    "tag_rpm_limit",
    "rpm_limit_type",
    "tpm_limit_type",
    "enforced_params",
    "temp_budget_increase",
    "temp_budget_expiry",
    "allowed_vector_store_indexes",
    "enforced_batch_output_expires_after",
    "enforced_file_expires_after",
    "throttle_on_budget_exceeded",
    "enable_prompt_caching",
    "end_user_budget_id",
];

const V2_FIELDS: [&str; 11] = [
    "organization_alias",
    "models",
    "metadata",
    "tpm_limit",
    "rpm_limit",
    "max_budget",
    "soft_budget",
    "max_parallel_requests",
    "model_max_budget",
    "budget_duration",
    "object_permission",
];

pub trait PresentFields {
    fn fields_set(&self) -> &BTreeSet<String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrganizationTableUpdate {
    #[serde(flatten)]
    pub budget: LiteLLMBudgetTable,
    pub organization_id: Option<String>,
    pub organization_alias: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub models: Option<Vec<String>>,
    pub updated_by: Option<String>,
    pub object_permission: Option<LiteLLMObjectPermissionBase>,
    pub model_tpm_limit: Option<BTreeMap<String, i64>>,
    pub model_rpm_limit: Option<BTreeMap<String, i64>>,
}

impl OrganizationTableUpdate {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let fields = set_model_info(object)?;
        let budget: LiteLLMBudgetTable = serde_json::from_value(Value::Object(fields.clone()))?;
        Ok(Self {
            budget,
            organization_id: field(&fields, "organization_id", Value::Null)?,
            organization_alias: field(&fields, "organization_alias", Value::Null)?,
            spend: field(&fields, "spend", Value::Null)?,
            metadata: field(&fields, "metadata", Value::Null)?,
            models: field(&fields, "models", Value::Null)?,
            updated_by: field(&fields, "updated_by", Value::Null)?,
            object_permission: field(&fields, "object_permission", Value::Null)?,
            model_tpm_limit: field(&fields, "model_tpm_limit", Value::Null)?,
            model_rpm_limit: field(&fields, "model_rpm_limit", Value::Null)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationUpdateRequestV2 {
    pub organization_alias: Option<String>,
    pub models: Option<Vec<String>>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub soft_budget: Option<Decimal>,
    pub max_parallel_requests: Option<i64>,
    pub model_max_budget: Option<BTreeMap<String, JsonData>>,
    pub budget_duration: Option<String>,
    pub object_permission: Option<LiteLLMObjectPermissionBase>,
    #[serde(skip)]
    fields_set: BTreeSet<String>,
}

impl OrganizationUpdateRequestV2 {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        reject_unknown_v2_fields(object)?;
        let mut request: Self = serde_json::from_value(value.clone())?;
        request.fields_set = object.keys().cloned().collect();
        Ok(request)
    }
}

impl PresentFields for OrganizationUpdateRequestV2 {
    fn fields_set(&self) -> &BTreeSet<String> {
        &self.fields_set
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OrganizationTableWithMembers {
    #[serde(flatten)]
    pub table: OrganizationTable,
    pub members: Vec<OrganizationMembership>,
    pub teams: Vec<LiteLLMTeamTable>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Deserialize)]
struct OrganizationMemberLists {
    #[serde(default)]
    members: Vec<OrganizationMembership>,
    #[serde(default)]
    teams: Vec<LiteLLMTeamTable>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl OrganizationTableWithMembers {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        let table = serde_json::from_value(Value::Object(object.clone()))?;
        let fields: OrganizationMemberLists =
            serde_json::from_value(Value::Object(object.clone()))?;
        Ok(Self::new(table, fields))
    }

    const fn new(table: OrganizationTable, fields: OrganizationMemberLists) -> Self {
        Self {
            table,
            members: fields.members,
            teams: fields.teams,
            created_at: fields.created_at,
            updated_at: fields.updated_at,
        }
    }
}

fn reject_unknown_v2_fields(object: &Map<String, Value>) -> Result<(), MemberContractError> {
    for name in object.keys() {
        if !V2_FIELDS.contains(&name.as_str()) {
            return Err(MemberContractError::Validation {
                message: "Extra inputs are not permitted".to_owned(),
            });
        }
    }
    Ok(())
}

fn set_model_info(object: &Map<String, Value>) -> Result<Map<String, Value>, MemberContractError> {
    let mut fields = object.clone();
    for name in MANAGEMENT_METADATA_FIELDS {
        let Some(value) = fields.get(name).filter(|raw| !raw.is_null()).cloned() else {
            continue;
        };
        if fields.get("metadata").is_none_or(Value::is_null) {
            fields.insert("metadata".to_owned(), Value::Object(Map::new()));
        }
        let Some(Value::Object(metadata)) = fields.get_mut("metadata") else {
            return Err(MemberContractError::Validation {
                message: "Input should be a valid dictionary".to_owned(),
            });
        };
        metadata.insert(name.to_owned(), value);
        fields.remove(name);
    }
    Ok(fields)
}

mod tests;
