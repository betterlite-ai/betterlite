use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::validate::{RecordError, Validate, check_items, check_length};

pub const MAX_EVIDENCE: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Support,
    Counterexample,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub execution_id: String,
    pub span_id: String,
    pub quote: String,
    #[serde(default)]
    pub role: Role,
}

impl Validate for Evidence {
    fn validate(&self) -> Result<(), RecordError> {
        check_length("quote", &self.quote, 1, 1000)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Issue,
    Pattern,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    #[default]
    Medium,
    Low,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingStatus {
    #[default]
    Open,
    Resolved,
    Dismissed,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingDraft {
    pub title: String,
    pub description: String,
    pub check_id: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub suggestion: String,
    #[serde(default)]
    pub limitation: String,
    pub evidence: Vec<Evidence>,
    // the worker names the finding it extends, or none for a new one
    #[serde(default)]
    pub existing_finding_id: Option<String>,
}

impl Validate for FindingDraft {
    fn validate(&self) -> Result<(), RecordError> {
        let notes = [self.suggestion.as_str(), self.limitation.as_str()];
        check_draft(&self.title, &self.description, notes, &self.evidence)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub title: String,
    pub description: String,
    pub check_id: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub suggestion: String,
    #[serde(default)]
    pub limitation: String,
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub existing_finding_id: Option<String>,
    pub id: String,
    #[serde(default)]
    pub status: FindingStatus,
    #[serde(default)]
    pub reason: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    #[serde(default)]
    pub occurrences: Vec<String>,
    pub revision: i64,
}

impl Validate for Finding {
    fn validate(&self) -> Result<(), RecordError> {
        let notes = [self.suggestion.as_str(), self.limitation.as_str()];
        check_draft(&self.title, &self.description, notes, &self.evidence)
    }
}

fn check_draft(
    title: &str,
    description: &str,
    [suggestion, limitation]: [&str; 2],
    evidence: &[Evidence],
) -> Result<(), RecordError> {
    check_length("title", title, 3, 160)?;
    check_length("description", description, 10, 4000)?;
    check_length("suggestion", suggestion, 0, 2000)?;
    check_length("limitation", limitation, 0, 600)?;
    check_items("evidence", evidence, 1, MAX_EVIDENCE)
}
