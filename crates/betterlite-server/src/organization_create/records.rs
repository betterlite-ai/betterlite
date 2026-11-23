use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{
    JsonData, LiteLLMBudgetTable, LiteLLMObjectPermissionTable, Timestamp,
};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::key_fields::values::LiteLLMObjectPermissionBase;
use crate::management_guards::amounts::Amount;

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

// a request money value: a JSON number read into Amount so non-finite values keep their description
#[derive(Debug, Clone, Copy)]
pub struct Money(pub Amount);

impl Money {
    #[must_use]
    pub fn decimal(self) -> Option<Decimal> {
        match self.0 {
            Amount::Finite(decimal) => Some(decimal),
            Amount::Nan | Amount::PositiveInfinity | Amount::NegativeInfinity => None,
        }
    }
}

impl<'de> Deserialize<'de> for Money {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let number = serde_json::Number::deserialize(deserializer)?;
        let Some(value) = number.as_f64() else {
            return Err(D::Error::custom("the value is not a JSON number"));
        };
        if value.is_nan() {
            return Ok(Self(Amount::Nan));
        }
        if value == f64::INFINITY {
            return Ok(Self(Amount::PositiveInfinity));
        }
        if value == f64::NEG_INFINITY {
            return Ok(Self(Amount::NegativeInfinity));
        }
        let Some(decimal) = Decimal::from_f64(value) else {
            return Err(D::Error::custom("the number is out of decimal range"));
        };
        Ok(Self(Amount::Finite(decimal)))
    }
}

impl Serialize for Money {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            Amount::Finite(decimal) => rust_decimal::serde::float::serialize(&decimal, serializer),
            Amount::Nan | Amount::PositiveInfinity | Amount::NegativeInfinity => Err(
                serde::ser::Error::custom("a non-finite amount cannot be stored"),
            ),
        }
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
    pub soft_budget: Option<Money>,
    #[serde(default)]
    pub max_budget: Option<Money>,
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
    pub temp_budget_increase: Option<Money>,
    #[serde(default)]
    pub temp_budget_expiry: Option<Timestamp>,
}

impl NewOrganizationRequest {
    // data.json(exclude_none=True): every sent field, with no null values and the permission's nulls dropped
    pub fn fields(&self) -> Result<BTreeMap<String, JsonData>, serde_json::Error> {
        let mut fields = BTreeMap::new();
        insert_option(&mut fields, "organization_id", &self.organization_id)?;
        insert_value(&mut fields, "organization_alias", &self.organization_alias)?;
        insert_value(&mut fields, "models", &self.models)?;
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
            let mut permission_data = to_json_data(permission)?;
            if let JsonData::Object(permission_fields) = &mut permission_data {
                permission_fields.retain(|_, value| !matches!(value, JsonData::Null));
            }
            fields.insert("object_permission".to_owned(), permission_data);
        }
        Ok(fields)
    }
}

fn insert_value<T>(
    fields: &mut BTreeMap<String, JsonData>,
    name: &str,
    value: &T,
) -> Result<(), serde_json::Error>
where
    T: Serialize + ?Sized,
{
    fields.insert(name.to_owned(), to_json_data(value)?);
    Ok(())
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
        insert_value(fields, name, inner)?;
    }
    Ok(())
}

pub fn to_json_data<T>(value: &T) -> Result<JsonData, serde_json::Error>
where
    T: Serialize + ?Sized,
{
    serde_json::from_str(&serde_json::to_string(value)?)
}

pub fn from_json_data<T>(value: &JsonData) -> Result<T, serde_json::Error>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(&serde_json::to_string(value)?)
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
