use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{
    JsonData, LiteLLMBudgetTable, LiteLLMObjectPermissionTable, Timestamp,
};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::key_fields::values::LiteLLMObjectPermissionBase;
use crate::management_guards::amounts::Amount;
use crate::organization_create::error::OrganizationCreateError;

// LiteLLM_BudgetTable's own fields, the allowlist a budget-row write filters the request through
pub const BUDGET_SCHEMA_FIELDS: [&str; 12] = [
    "budget_id",
    "soft_budget",
    "max_budget",
    "max_parallel_requests",
    "tpm_limit",
    "rpm_limit",
    "tpd_limit",
    "model_max_budget",
    "budget_duration",
    "allowed_models",
    "temp_budget_increase",
    "temp_budget_expiry",
];

// a request money value as Amount: a number that overflows f64 keeps the inf or -inf spelling
#[must_use]
pub fn amount_of(number: &Number) -> Option<Amount> {
    let value = number.as_f64()?;
    if value.is_nan() {
        return Some(Amount::Nan);
    }
    if value == f64::INFINITY {
        return Some(Amount::PositiveInfinity);
    }
    if value == f64::NEG_INFINITY {
        return Some(Amount::NegativeInfinity);
    }
    Decimal::from_f64(value).map(Amount::Finite)
}

// a sent money value: None is absent, and a number past Decimal's range is refused since no budget row could hold it
pub fn request_amount(
    field: &'static str,
    value: Option<&Number>,
) -> Result<Option<Amount>, OrganizationCreateError> {
    let Some(number) = value else {
        return Ok(None);
    };
    match amount_of(number) {
        Some(amount) => Ok(Some(amount)),
        None => Err(OrganizationCreateError::BudgetRange {
            field,
            received: number.to_string(),
        }),
    }
}

// the Decimal a budget response column carries; a non-finite amount never reaches storage
#[must_use]
pub fn decimal_of(amount: Amount) -> Option<Decimal> {
    match amount {
        Amount::Finite(decimal) => Some(decimal),
        Amount::Nan | Amount::PositiveInfinity | Amount::NegativeInfinity => None,
    }
}

// proxy/_types.py NewOrganizationRequest: LiteLLM_BudgetTable plus the organization fields
#[derive(Debug, Clone, Deserialize)]
pub struct NewOrganizationRequest {
    #[serde(default)]
    pub organization_id: Option<String>,
    pub organization_alias: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub model_rpm_limit: Option<BTreeMap<String, i64>>,
    #[serde(default)]
    pub model_tpm_limit: Option<BTreeMap<String, i64>>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionBase>,
    #[serde(default)]
    pub soft_budget: Option<Number>,
    #[serde(default)]
    pub max_budget: Option<Number>,
    #[serde(default)]
    pub max_parallel_requests: Option<i64>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub tpd_limit: Option<i64>,
    #[serde(default)]
    pub model_max_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    #[serde(default)]
    pub temp_budget_increase: Option<Number>,
    #[serde(default)]
    pub temp_budget_expiry: Option<Timestamp>,
}

impl NewOrganizationRequest {
    // data.json(exclude_none=True): every sent field, with no null values and the permission's nulls dropped
    pub fn fields(&self) -> Result<BTreeMap<String, JsonData>, serde_json::Error> {
        let mut fields = BTreeMap::new();
        insert_option(&mut fields, "organization_id", &self.organization_id)?;
        fields.insert(
            "organization_alias".to_owned(),
            serde_json::from_value::<JsonData>(serde_json::to_value(&self.organization_alias)?)?,
        );
        fields.insert(
            "models".to_owned(),
            serde_json::from_value::<JsonData>(serde_json::to_value(&self.models)?)?,
        );
        insert_option(&mut fields, "budget_id", &self.budget_id)?;
        insert_option(&mut fields, "metadata", &self.metadata)?;
        insert_option(&mut fields, "model_rpm_limit", &self.model_rpm_limit)?;
        insert_option(&mut fields, "model_tpm_limit", &self.model_tpm_limit)?;
        insert_option(&mut fields, "soft_budget", &self.soft_budget)?;
        insert_option(&mut fields, "max_budget", &self.max_budget)?;
        insert_option(
            &mut fields,
            "max_parallel_requests",
            &self.max_parallel_requests,
        )?;
        insert_option(&mut fields, "tpm_limit", &self.tpm_limit)?;
        insert_option(&mut fields, "rpm_limit", &self.rpm_limit)?;
        insert_option(&mut fields, "tpd_limit", &self.tpd_limit)?;
        insert_option(&mut fields, "model_max_budget", &self.model_max_budget)?;
        insert_option(&mut fields, "budget_duration", &self.budget_duration)?;
        insert_option(&mut fields, "allowed_models", &self.allowed_models)?;
        insert_option(
            &mut fields,
            "temp_budget_increase",
            &self.temp_budget_increase,
        )?;
        insert_option(&mut fields, "temp_budget_expiry", &self.temp_budget_expiry)?;
        if let Some(permission) = &self.object_permission {
            let mut permission_data =
                serde_json::from_value::<JsonData>(serde_json::to_value(permission)?)?;
            if let JsonData::Object(permission_fields) = &mut permission_data {
                permission_fields.retain(|_, value| !matches!(value, JsonData::Null));
            }
            fields.insert("object_permission".to_owned(), permission_data);
        }
        Ok(fields)
    }
}

fn insert_option<T>(
    fields: &mut BTreeMap<String, JsonData>,
    name: &str,
    value: &Option<T>,
) -> Result<(), serde_json::Error>
where
    T: Serialize + ?Sized,
{
    if let Some(inner) = value {
        fields.insert(
            name.to_owned(),
            serde_json::from_value::<JsonData>(serde_json::to_value(inner)?)?,
        );
    }
    Ok(())
}

// NewOrganizationResponse: B723's organization row with the three required creation columns
#[derive(Debug, Clone, Serialize)]
pub struct NewOrganizationResponse {
    pub organization_id: String,
    pub organization_alias: String,
    pub budget_id: String,
    #[serde(serialize_with = "rust_decimal::serde::float::serialize")]
    pub spend: Decimal,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub models: Vec<String>,
    pub model_spend: BTreeMap<String, JsonData>,
    pub created_by: String,
    pub updated_by: String,
    pub users: Option<Vec<LiteLLMUserTable>>,
    pub litellm_budget_table: Option<LiteLLMBudgetTable>,
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
    pub object_permission_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

// the columns a created organization row comes back with; the relations are read or composed separately
#[derive(Debug, Clone, Deserialize)]
pub struct OrganizationRow {
    pub organization_id: String,
    pub organization_alias: String,
    pub budget_id: String,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    pub created_by: String,
    pub updated_by: String,
    #[serde(default)]
    pub object_permission_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
