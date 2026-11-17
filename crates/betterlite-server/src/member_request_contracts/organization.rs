use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::{
    JsonData, LiteLLMBudgetTable, OrganizationMembership, Timestamp,
};
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::key_fields::values::LiteLLMObjectPermissionBase;

use super::{MemberContractError, defaulted_field, json_object, optional_field, required_field};

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
            organization_id: optional_field(&fields, "organization_id")?,
            organization_alias: optional_field(&fields, "organization_alias")?,
            spend: optional_field(&fields, "spend")?,
            metadata: optional_field(&fields, "metadata")?,
            models: optional_field(&fields, "models")?,
            updated_by: optional_field(&fields, "updated_by")?,
            object_permission: optional_field(&fields, "object_permission")?,
            model_tpm_limit: optional_field(&fields, "model_tpm_limit")?,
            model_rpm_limit: optional_field(&fields, "model_rpm_limit")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrganizationUpdateRequestV2 {
    pub organization_alias: Option<String>,
    pub models: Option<Vec<String>>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
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
        for name in object.keys() {
            if !V2_FIELDS.contains(&name.as_str()) {
                return Err(MemberContractError::Validation {
                    message: "Extra inputs are not permitted".to_owned(),
                });
            }
        }
        Ok(Self {
            organization_alias: optional_field(object, "organization_alias")?,
            models: optional_field(object, "models")?,
            metadata: optional_field(object, "metadata")?,
            tpm_limit: optional_field(object, "tpm_limit")?,
            rpm_limit: optional_field(object, "rpm_limit")?,
            max_budget: optional_field(object, "max_budget")?,
            soft_budget: optional_field(object, "soft_budget")?,
            max_parallel_requests: optional_field(object, "max_parallel_requests")?,
            model_max_budget: optional_field(object, "model_max_budget")?,
            budget_duration: optional_field(object, "budget_duration")?,
            object_permission: optional_field(object, "object_permission")?,
            fields_set: object.keys().cloned().collect(),
        })
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

impl OrganizationTableWithMembers {
    pub fn from_value(value: &Value) -> Result<Self, MemberContractError> {
        let object = json_object(value)?;
        Ok(Self {
            table: serde_json::from_value(value.clone())?,
            members: defaulted_field(object, "members")?,
            teams: defaulted_field(object, "teams")?,
            created_at: required_field(object, "created_at")?,
            updated_at: required_field(object, "updated_at")?,
        })
    }
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
