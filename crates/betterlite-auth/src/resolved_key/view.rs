use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::team_rows::member::Member;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::resolved_key::materialize;

mod tests;

const BUDGET_TABLE_PREFIX: &str = "litellm_budget_table_";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationTokenView {
    #[serde(flatten)]
    pub token: LiteLLMVerificationToken,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_spend: Option<Decimal>,
    #[serde(default)]
    pub team_alias: Option<String>,
    #[serde(default)]
    pub team_tpm_limit: Option<i64>,
    #[serde(default)]
    pub team_rpm_limit: Option<i64>,
    #[serde(default)]
    pub team_tpd_limit: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_max_budget: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_soft_budget: Option<Decimal>,
    #[serde(default)]
    pub team_model_max_budget: Option<Payload>,
    #[serde(default)]
    pub team_models: Vec<JsonData>,
    #[serde(default)]
    pub team_blocked: bool,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub soft_budget: Option<Decimal>,
    #[serde(default)]
    pub team_model_aliases: Option<Payload>,
    #[serde(default)]
    pub team_member: Option<Member>,
    #[serde(default)]
    pub team_metadata: Option<Payload>,
    #[serde(default)]
    pub team_object_permission_id: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_member_spend: Option<Decimal>,
    #[serde(default)]
    pub team_member_tpm_limit: Option<i64>,
    #[serde(default)]
    pub team_member_rpm_limit: Option<i64>,
    #[serde(default)]
    pub end_user_id: Option<String>,
    #[serde(default)]
    pub end_user_tpm_limit: Option<i64>,
    #[serde(default)]
    pub end_user_rpm_limit: Option<i64>,
    #[serde(default)]
    pub end_user_tpd_limit: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub end_user_max_budget: Option<Decimal>,
    #[serde(default)]
    pub end_user_model_max_budget: Option<Payload>,
    #[serde(default)]
    pub organization_alias: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub organization_max_budget: Option<Decimal>,
    #[serde(default)]
    pub organization_tpm_limit: Option<i64>,
    #[serde(default)]
    pub organization_rpm_limit: Option<i64>,
    #[serde(default)]
    pub organization_metadata: Option<Payload>,
    #[serde(default)]
    pub project_alias: Option<String>,
    #[serde(default)]
    pub project_metadata: Option<Payload>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub last_refreshed_at: Option<Decimal>,
}

#[derive(Debug, thiserror::Error)]
pub enum ViewError {
    #[error("failed to decode a verification token view")]
    Json(#[source] serde_json::Error),
}

impl VerificationTokenView {
    pub fn construct(mut fields: BTreeMap<String, JsonData>) -> Result<Self, ViewError> {
        normalize_fields(&mut fields);
        materialize(fields).map_err(ViewError::Json)
    }
}

pub(crate) fn normalize_fields(fields: &mut BTreeMap<String, JsonData>) {
    let original: Vec<(String, JsonData)> = fields
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    for (key, value) in &original {
        if let Some(attr_name) = budget_table_target(key, value) {
            apply_budget_value(fields, &attr_name, value.clone());
        }
        if key == "end_user_id"
            && let Some(text) = stringify_end_user_id(value)
        {
            fields.insert(key.clone(), JsonData::String(text));
        }
    }

    override_organization_id(fields);
}

fn budget_table_target(key: &str, value: &JsonData) -> Option<String> {
    if matches!(value, JsonData::Null) || !key.starts_with(BUDGET_TABLE_PREFIX) {
        return None;
    }
    Some(key.replace(BUDGET_TABLE_PREFIX, ""))
}

fn apply_budget_value(fields: &mut BTreeMap<String, JsonData>, attr_name: &str, value: JsonData) {
    let should_apply = match fields.get(attr_name) {
        None | Some(JsonData::Null) => true,
        Some(JsonData::Object(current)) => attr_name == "model_max_budget" && current.is_empty(),
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Array(_),
        ) => false,
    };
    if should_apply {
        fields.insert(attr_name.to_owned(), value);
    }
}

fn stringify_end_user_id(value: &JsonData) -> Option<String> {
    match value {
        JsonData::Bool(flag) => Some(if *flag { "True" } else { "False" }.to_owned()),
        JsonData::Number(number) if number.is_i64() || number.is_u64() => Some(number.to_string()),
        JsonData::Number(_)
        | JsonData::Null
        | JsonData::String(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}

fn override_organization_id(fields: &mut BTreeMap<String, JsonData>) {
    if matches!(fields.get("organization_id"), None | Some(JsonData::Null)) {
        return;
    }
    if let Some(value) = fields.remove("organization_id") {
        fields.insert("org_id".to_owned(), value);
    }
}
