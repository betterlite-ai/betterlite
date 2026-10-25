use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIRepository {
    pub name: String,
    pub visibility: String,
    pub archived: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIRepositoriesResponse {
    pub repositories: Vec<ROIRepository>,
    pub page: u32,
    pub has_more: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROISyncStatus {
    pub running: bool,
    pub phase: ROISyncPhase,
    pub stage: String,
    pub done: u32,
    pub total: u32,
    pub estimated: u32,
    pub reused: u32,
    pub needs_attention: u32,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub next_update: Option<String>,
    pub elapsed_seconds: u32,
    pub remaining_seconds: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROISpendRecord {
    pub date: String,
    pub user_id: String,
    pub email: String,
    pub spend: Decimal,
    pub requests: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIEstimate {
    pub status: ROIEstimateStatus,
    pub hours: Option<f64>,
    pub reasoning: String,
    pub model: Option<String>,
    pub evidence_source: Option<String>,
    pub effort_basis: Option<String>,
    pub cached: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ROIEstimateStatus {
    Estimated,
    NeedsReview,
    Error,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIPullRecord {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub login: String,
    pub emails: Vec<String>,
    pub profile_email: String,
    pub commit_emails: Option<Vec<String>>,
    pub merged_at: String,
    pub head_sha: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub commit_count: u64,
    pub incomplete_metadata: bool,
    pub estimate: ROIEstimate,
    pub cache_key: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub warnings: Option<Vec<String>>,
    pub unavailable_repos: Option<Vec<String>>,
    pub id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIPullFile {
    pub filename: Option<String>,
    pub status: Option<String>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIPullCommit {
    pub sha: String,
    pub message: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub changed_files: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIPullEvidence {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub login: String,
    pub emails: Vec<String>,
    pub profile_email: String,
    pub commit_emails: Option<Vec<String>>,
    pub merged_at: String,
    pub head_sha: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub files: Vec<ROIPullFile>,
    pub commits: Vec<ROIPullCommit>,
    pub commit_count: u64,
    pub incomplete_metadata: bool,
}
#[cfg(test)]
mod tests;
