use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSourceConfigCustom {
    pub item_schema: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_sample_schema: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSourceConfigLogs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSourceStoredCompletions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DataSourceConfig {
    Custom(DataSourceConfigCustom),
    Logs(DataSourceConfigLogs),
    StoredCompletions(DataSourceStoredCompletions),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LLMAsJudgeGraderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundTruthMetric {
    ExactMatch,
    F1Score,
    Bleu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundTruthGraderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<GroundTruthMetric>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomGraderConfig {
    pub function_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GraderConfig {
    LlmAsJudge(LLMAsJudgeGraderConfig),
    GroundTruth(GroundTruthGraderConfig),
    Custom(CustomGraderConfig),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateEvalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub data_source_config: DataSourceConfig,
    pub testing_criteria: Vec<GraderConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateEvalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalOrderBy {
    CreatedAt,
    UpdatedAt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEvalsParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<EvalOrder>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_by: Option<EvalOrderBy>,
}

fn default_eval_object() -> String {
    "eval".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eval {
    pub id: String,
    #[serde(default = "default_eval_object")]
    pub object: String,
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub name: Option<String>,
    pub data_source_config: Map<String, Value>,
    pub testing_criteria: Vec<Map<String, Value>>,
    pub metadata: Option<Map<String, Value>>,
}

fn default_list_object() -> String {
    "list".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEvalsResponse {
    #[serde(default = "default_list_object")]
    pub object: String,
    pub data: Vec<Eval>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

fn default_eval_deleted_object() -> String {
    "eval.deleted".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteEvalResponse {
    pub eval_id: String,
    #[serde(default = "default_eval_deleted_object")]
    pub object: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelledStatus {
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelEvalResponse {
    pub id: String,
    #[serde(default = "default_eval_object")]
    pub object: String,
    pub status: CancelledStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRunRequest {
    pub data_source: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRunsParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<EvalOrder>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultCounts {
    pub total: i64,
    #[serde(default)]
    pub passed: i64,
    #[serde(default)]
    pub failed: i64,
    #[serde(default)]
    pub error: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PerTestingCriteriaResult {
    pub testing_criteria_index: i64,
    pub result_counts: ResultCounts,
    pub average_score: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

fn default_run_object() -> String {
    "eval.run".to_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    #[serde(default = "default_run_object")]
    pub object: String,
    pub created_at: i64,
    pub status: RunStatus,
    pub data_source: Map<String, Value>,
    pub eval_id: String,
    pub name: Option<String>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub model: Option<String>,
    pub per_model_usage: Option<Value>,
    pub per_testing_criteria_results: Option<Vec<PerTestingCriteriaResult>>,
    pub report_url: Option<String>,
    pub result_counts: Option<BTreeMap<String, i64>>,
    pub shared_with_openai: Option<bool>,
    pub metadata: Option<Map<String, Value>>,
    pub error: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListRunsResponse {
    #[serde(default = "default_list_object")]
    pub object: String,
    pub data: Vec<Run>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRunResponse {
    pub id: String,
    #[serde(default = "default_run_object")]
    pub object: String,
    pub status: CancelledStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunDeletedObject(pub Option<String>);

impl Default for RunDeletedObject {
    fn default() -> Self {
        Self(Some("eval.run.deleted".to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunDeleted(pub Option<bool>);

impl Default for RunDeleted {
    fn default() -> Self {
        Self(Some(true))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDeleteResponse {
    pub run_id: String,
    #[serde(default)]
    pub object: RunDeletedObject,
    #[serde(default)]
    pub deleted: RunDeleted,
}
