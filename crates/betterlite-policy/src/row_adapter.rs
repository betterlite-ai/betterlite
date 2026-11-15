mod tests;

use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::pipeline::json_value::JsonValue;
use crate::records::JsonObject;
use crate::records::db::PolicyDBResponse;
use crate::registry::PolicySource;

pub type PolicyClause = BTreeMap<String, JsonValue>;
pub type PolicyOrder = BTreeMap<String, PolicySort>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicySort {
    Asc,
    Desc,
}

// Prisma types the two Json columns as str, yet a read hands back the decoded mapping
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PolicyVersionJson {
    Object(JsonObject),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyRow {
    pub policy_id: String,
    pub policy_name: String,
    #[serde(default)]
    pub version_number: Option<i32>,
    #[serde(default)]
    pub version_status: Option<String>,
    #[serde(default)]
    pub parent_version_id: Option<String>,
    #[serde(default)]
    pub is_latest: Option<bool>,
    #[serde(default)]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub production_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub inherit: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub guardrails_add: Option<Vec<String>>,
    #[serde(default)]
    pub guardrails_remove: Option<Vec<String>>,
    #[serde(default)]
    pub condition: Option<JsonObject>,
    #[serde(default)]
    pub pipeline: Option<JsonObject>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyVersionSourceRow {
    pub policy_id: String,
    pub policy_name: String,
    pub version_number: i32,
    #[serde(default)]
    pub inherit: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub guardrails_add: Option<Vec<String>>,
    #[serde(default)]
    pub guardrails_remove: Option<Vec<String>>,
    #[serde(default)]
    pub condition: Option<PolicyVersionJson>,
    #[serde(default)]
    pub pipeline: Option<PolicyVersionJson>,
}

pub trait PolicyTable<Row: Send + 'static>: Send + Sync {
    type Failure: Error + Send + Sync + 'static;

    fn create(&self, data: PolicyClause)
    -> impl Future<Output = Result<Row, Self::Failure>> + Send;

    fn find_unique(
        &self,
        filter: PolicyClause,
    ) -> impl Future<Output = Result<Option<Row>, Self::Failure>> + Send;

    fn find_many(
        &self,
        filter: Option<PolicyClause>,
        order: Option<PolicyOrder>,
    ) -> impl Future<Output = Result<Vec<Row>, Self::Failure>> + Send;

    fn update(
        &self,
        filter: PolicyClause,
        data: PolicyClause,
    ) -> impl Future<Output = Result<Row, Self::Failure>> + Send;

    fn update_many(
        &self,
        filter: PolicyClause,
        data: PolicyClause,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send;

    fn delete(
        &self,
        filter: PolicyClause,
    ) -> impl Future<Output = Result<Option<Row>, Self::Failure>> + Send;

    fn delete_many(
        &self,
        filter: PolicyClause,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send;
}

pub trait PolicyTableSource: Send + Sync {
    type Table: PolicyTable<PolicyRow>;

    fn policy_table(&self) -> Self::Table;
}

pub trait PolicyVersionTableSource: Send + Sync {
    type Table: PolicyTable<PolicyVersionSourceRow>;

    fn policy_version_source_table(&self) -> Self::Table;
}

#[must_use]
pub fn policy_db_response(row: &PolicyRow) -> PolicyDBResponse {
    PolicyDBResponse {
        policy_id: row.policy_id.clone(),
        policy_name: row.policy_name.clone(),
        version_number: row.version_number.unwrap_or(1),
        version_status: row
            .version_status
            .clone()
            .unwrap_or_else(|| "production".to_owned()),
        parent_version_id: row.parent_version_id.clone(),
        is_latest: row.is_latest.unwrap_or(true),
        published_at: row.published_at,
        production_at: row.production_at,
        inherit: row.inherit.clone(),
        description: row.description.clone(),
        guardrails_add: row.guardrails_add.clone().unwrap_or_default(),
        guardrails_remove: row.guardrails_remove.clone().unwrap_or_default(),
        condition: row.condition.clone(),
        pipeline: row.pipeline.clone(),
        created_at: Some(row.created_at),
        updated_at: Some(row.updated_at),
        created_by: row.created_by.clone(),
        updated_by: row.updated_by.clone(),
        definition_location: PolicySource::Db,
    }
}
