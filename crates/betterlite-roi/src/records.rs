use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ROIRepository {
    pub name: String,
    pub visibility: String,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ROIRepositoriesResponse {
    pub repositories: Vec<ROIRepository>,
    pub page: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ROISyncPhase {
    Idle,
    Spend,
    Repositories,
    Estimates,
    Complete,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ROISyncStatus {
    pub running: bool,
    pub phase: ROISyncPhase,
    pub stage: String,
    pub done: i64,
    pub total: i64,
    pub estimated: i64,
    pub reused: i64,
    pub needs_attention: i64,
    #[serde(deserialize_with = "Option::deserialize")]
    pub error: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub next_update: Option<String>,
    #[serde(default)]
    pub elapsed_seconds: i64,
    #[serde(default)]
    pub remaining_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ROISpendRecord {
    pub date: String,
    pub user_id: String,
    pub email: String,
    #[serde(with = "rust_decimal::serde::float")]
    pub spend: Decimal,
    pub requests: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ROIEstimateStatus {
    Estimated,
    NeedsReview,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ROIEstimate {
    pub status: ROIEstimateStatus,
    #[serde(deserialize_with = "Option::deserialize")]
    pub hours: Option<f64>,
    pub reasoning: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ROIPullRecord {
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub login: String,
    pub emails: Vec<String>,
    pub profile_email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_emails: Option<Vec<String>>,
    pub merged_at: String,
    pub head_sha: String,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub commit_count: i64,
    pub incomplete_metadata: bool,
    pub estimate: ROIEstimate,
    #[serde(deserialize_with = "Option::deserialize")]
    pub cache_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ROIReport {
    pub mode: String,
    pub start: String,
    pub end: String,
    pub synced_at: String,
    pub repos: Vec<String>,
    pub estimator_model: String,
    pub estimator_prompt: String,
    pub effort_basis: String,
    pub spend: Vec<ROISpendRecord>,
    pub pulls: Vec<ROIPullRecord>,
    pub settings_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_repos: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ROIPullFile {
    #[serde(deserialize_with = "Option::deserialize")]
    pub filename: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub status: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub additions: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub deletions: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ROIPullCommit {
    pub sha: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<i64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub changed_files: Option<Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ROIPullEvidence {
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub login: String,
    pub emails: Vec<String>,
    pub profile_email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_emails: Option<Vec<String>>,
    pub merged_at: String,
    pub head_sha: String,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub files: Vec<ROIPullFile>,
    pub commits: Vec<ROIPullCommit>,
    pub commit_count: i64,
    pub incomplete_metadata: bool,
}

mod tests;
