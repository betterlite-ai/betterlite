use indexmap::IndexSet;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::records::ROIEstimateStatus;
use crate::summary::identity::MatchMethod;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ROIEstimateResponse {
    pub status: ROIEstimateStatus,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub hours: Option<Decimal>,
    pub reasoning: String,
    pub model: Option<String>,
    pub evidence_source: Option<String>,
    pub effort_basis: Option<String>,
    pub cached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ROIPullSummary {
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub login: String,
    pub emails: Vec<String>,
    pub profile_email: String,
    pub merged_at: String,
    pub head_sha: String,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub commit_count: i64,
    pub incomplete_metadata: bool,
    pub estimate: ROIEstimateResponse,
    pub cache_key: Option<String>,
    pub email: String,
    pub match_method: MatchMethod,
    pub matched: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ROIPersonSummary {
    pub id: String,
    pub email: String,
    pub logins: IndexSet<String>,
    // absent means the person has no gateway spend rows, which keeps them out of the cohort
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub hours: Decimal,
    pub prs: usize,
    pub estimated_prs: usize,
    pub pending_prs: usize,
    pub match_methods: IndexSet<MatchMethod>,
    pub eligible: bool,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cost_per_hour: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ROISummaryMetrics {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub matched_spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub output_hours: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_output_hours: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub excluded_spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cost_per_hour: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub hours_per_dollar: Option<Decimal>,
    pub merged_prs: usize,
    pub estimated_prs: usize,
    pub matched_prs: usize,
    pub cohort_people: usize,
    pub people_with_prs: usize,
    pub pending_prs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ROITrendDay {
    pub date: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub hours: Decimal,
    pub prs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ROISummary {
    pub id: Option<String>,
    pub mode: String,
    pub start: String,
    pub end: String,
    pub synced_at: String,
    pub repos: Vec<String>,
    pub estimator_model: String,
    pub estimator_prompt: String,
    pub warnings: Vec<String>,
    pub effort_basis: String,
    pub metrics: ROISummaryMetrics,
    pub people: Vec<ROIPersonSummary>,
    pub pulls: Vec<ROIPullSummary>,
    pub trend: Vec<ROITrendDay>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ROIReportResponse {
    pub report: Option<ROISummary>,
}
