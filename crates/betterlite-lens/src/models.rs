use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;

pub trait Record: for<'de> Deserialize<'de> + Serialize {
    fn validate(&self) -> Result<(), String>;
}
pub fn decode<T>(bytes: &[u8]) -> Result<T, String>
where
    T: Record,
{
    let record: T = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    record.validate()?;
    Ok(record)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub team_id: String,
    pub api_key_hash: String,
    pub all_teams: bool,
}
impl Record for Scope {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataFilter {
    pub key: String,
    pub value: String,
}
impl Record for MetadataFilter {
    fn validate(&self) -> Result<(), String> {
        bound(&self.key, 1, 200, "key")?;
        bound(&self.value, 1, 500, "value")
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub instruction: String,
    pub enabled: bool,
}
impl Record for Check {
    fn validate(&self) -> Result<(), String> {
        bound(&self.id, 1, 80, "id")?;
        bound(&self.instruction, 3, 3000, "instruction")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LensSettings {
    pub name: String,
    #[serde(default)]
    pub context: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default = "default_lookback")]
    pub lookback_hours: u32,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub agent_name: String,
    #[serde(default)]
    pub filters: Vec<MetadataFilter>,
    #[serde(default)]
    pub checks: Vec<Check>,
    pub model: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_interval")]
    pub interval_minutes: u32,
    #[serde(default)]
    pub sample_size: Option<u32>,
    #[serde(default = "default_percent")]
    pub sample_percent: Decimal,
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub execution_ids: Vec<String>,
    #[serde(default = "default_budget")]
    pub monthly_budget: Decimal,
}
impl Record for LensSettings {
    fn validate(&self) -> Result<(), String> {
        bound(&self.name, 1, 100, "name")?;
        bound(&self.context, 0, 6000, "context")?;
        bound(&self.service, 0, 200, "service")?;
        bound(&self.agent_name, 0, 200, "agent_name")?;
        bound(&self.model, 1, 200, "model")?;
        if !matches!(self.source.as_str(), "traces" | "requests" | "both")
            || !(1..=8760).contains(&self.lookback_hours)
            || !(1..=10080).contains(&self.interval_minutes)
            || self.concurrency == 0
        {
            return Err("invalid settings range".to_owned());
        }
        if self.filters.len() > 8 {
            return Err("too many metadata filters".to_owned());
        }
        for filter in &self.filters {
            filter.validate()?;
        }
        for check in &self.checks {
            check.validate()?;
        }
        if self.sample_size == Some(0)
            || self.sample_percent <= Decimal::ZERO
            || self.sample_percent > Decimal::from(100_i32)
            || self.monthly_budget <= Decimal::ZERO
            || self.monthly_budget > Decimal::from(100_000_i32)
        {
            return Err("invalid sampling or budget range".to_owned());
        }
        let mut ids = BTreeSet::new();
        if self.checks.iter().any(|check| !ids.insert(&check.id)) {
            return Err("Each check must have a unique ID".to_owned());
        }
        if self.context.trim().is_empty() && !self.checks.iter().any(|check| check.enabled) {
            return Err("Describe expected behavior or add an enabled check".to_owned());
        }
        if self
            .checks
            .iter()
            .any(|check| check.id == "expected_behavior")
        {
            return Err("expected_behavior is reserved for the behavior description".to_owned());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub execution_id: String,
    pub span_id: String,
    pub quote: String,
    #[serde(default = "default_support")]
    pub role: String,
}
impl Record for Evidence {
    fn validate(&self) -> Result<(), String> {
        bound(&self.quote, 1, 1000, "quote")?;
        if matches!(self.role.as_str(), "support" | "counterexample") {
            Ok(())
        } else {
            Err("invalid role".to_owned())
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingDraft {
    pub title: String,
    pub description: String,
    pub check_id: String,
    #[serde(default = "default_issue")]
    pub kind: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub suggestion: String,
    #[serde(default)]
    pub limitation: String,
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub existing_finding_id: Option<String>,
}
impl Record for FindingDraft {
    fn validate(&self) -> Result<(), String> {
        bound(&self.title, 3, 160, "title")?;
        bound(&self.description, 10, 4000, "description")?;
        bound(&self.suggestion, 0, 2000, "suggestion")?;
        bound(&self.limitation, 0, 600, "limitation")?;
        if !matches!(self.kind.as_str(), "issue" | "pattern")
            || !matches!(self.priority.as_str(), "high" | "medium" | "low")
            || self.evidence.is_empty()
            || self.evidence.len() > 20
        {
            return Err("invalid finding draft".to_owned());
        }
        for evidence in &self.evidence {
            evidence.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub title: String,
    pub description: String,
    pub check_id: String,
    pub kind: String,
    pub priority: String,
    pub suggestion: String,
    pub limitation: String,
    pub evidence: Vec<Evidence>,
    pub existing_finding_id: Option<String>,
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub reason: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    #[serde(default)]
    pub occurrences: Vec<String>,
    pub revision: u64,
}
impl Record for Finding {
    fn validate(&self) -> Result<(), String> {
        if matches!(self.status.as_str(), "open" | "resolved" | "dismissed") {
            Ok(())
        } else {
            Err("invalid finding status".to_owned())
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    #[serde(default)]
    pub eligible: u64,
    #[serde(default)]
    pub selected: u64,
    #[serde(default)]
    pub screened: u64,
    #[serde(default)]
    pub investigated: u64,
    #[serde(default)]
    pub inconclusive: u64,
    #[serde(default)]
    pub grouping_batches: u64,
    #[serde(default)]
    pub grouped_batches: u64,
    #[serde(default)]
    pub candidates: u64,
    #[serde(default)]
    pub partial: u64,
    #[serde(default)]
    pub unassessable: u64,
}
impl Record for Coverage {
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    pub id: String,
    pub source: String,
    pub trace_id: String,
    #[serde(default)]
    pub trace_ref: String,
    pub team_id: String,
    pub name: String,
    pub start_time: String,
    pub span_count: u64,
    #[serde(default)]
    pub root_seen: bool,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub metadata: Vec<MetadataFilter>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub executions: Vec<Execution>,
    pub eligible: u64,
    #[serde(default)]
    pub selected: u64,
    #[serde(default)]
    pub next_offset: Option<u64>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunAssessment {
    pub execution_id: String,
    #[serde(default)]
    pub issue_checks: Vec<String>,
    #[serde(default)]
    pub pattern_checks: Vec<String>,
    #[serde(default)]
    pub cannot_assess: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub id: String,
    #[serde(default = "default_queued")]
    pub status: String,
    #[serde(default = "default_queued_stage")]
    pub stage: String,
    pub created_at: DateTime<Utc>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub settings: LensSettings,
    pub revision: u64,
    #[serde(default)]
    pub worker_id: Option<String>,
    #[serde(default)]
    pub lease_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub coverage: Coverage,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub sample: Option<Sample>,
    #[serde(default)]
    pub cost: Decimal,
    #[serde(default)]
    pub findings: Option<Vec<Finding>>,
    #[serde(default)]
    pub assessments: Vec<RunAssessment>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lens {
    pub id: String,
    pub scope: Scope,
    pub settings: LensSettings,
    #[serde(default = "default_revision")]
    pub revision: u64,
    #[serde(default)]
    pub version: u64,
    pub created_at: DateTime<Utc>,
    pub next_run_at: DateTime<Utc>,
    #[serde(default)]
    pub last_scan_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub findings: Vec<Finding>,
    pub budget_month: String,
    #[serde(default)]
    pub spent: Decimal,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Worker {
    #[serde(default)]
    pub analysis_key_id: Option<String>,
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub last_seen: DateTime<Utc>,
    #[serde(default)]
    pub revoked: bool,
}
impl Record for Job {
    fn validate(&self) -> Result<(), String> {
        self.settings.validate()
    }
}
impl Record for Lens {
    fn validate(&self) -> Result<(), String> {
        self.settings.validate()
    }
}
impl Record for Worker {
    fn validate(&self) -> Result<(), String> {
        if self
            .analysis_key_id
            .as_ref()
            .is_some_and(|key| key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            Err("invalid analysis key ID".to_owned())
        } else {
            Ok(())
        }
    }
}
#[must_use]
pub fn finding_identity(lens_id: &str, check_id: &str, kind: &str, title: &str) -> String {
    digest(&format!(
        "{lens_id}:{check_id}:{kind}:{}",
        title.to_lowercase()
    ))
}
#[must_use]
pub fn legacy_finding_identity(lens_id: &str, check_id: &str, title: &str) -> String {
    digest(&format!("{lens_id}:{check_id}:{}", title.to_lowercase()))
}
fn digest(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    let mut result = String::with_capacity(24);
    for byte in digest.iter().take(12) {
        let _ = write!(result, "{byte:02x}");
    }
    result
}
fn bound(value: &str, minimum: usize, maximum: usize, name: &str) -> Result<(), String> {
    if value.len() < minimum || value.len() > maximum {
        Err(format!("{name} length is outside its bounds"))
    } else {
        Ok(())
    }
}
fn default_source() -> String {
    "traces".to_owned()
}
const fn default_lookback() -> u32 {
    24
}
const fn default_true() -> bool {
    true
}
const fn default_interval() -> u32 {
    15
}
fn default_percent() -> Decimal {
    Decimal::from(100)
}
const fn default_concurrency() -> u32 {
    8
}
fn default_budget() -> Decimal {
    Decimal::from(100)
}
fn default_support() -> String {
    "support".to_owned()
}
fn default_issue() -> String {
    "issue".to_owned()
}
fn default_priority() -> String {
    "medium".to_owned()
}
fn default_queued() -> String {
    "queued".to_owned()
}
fn default_queued_stage() -> String {
    "Queued".to_owned()
}
const fn default_revision() -> u64 {
    1
}
