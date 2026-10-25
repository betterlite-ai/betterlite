use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};

pub const MAX_EVIDENCE: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Support,
    Counterexample,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Evidence {
    pub execution_id: String,
    pub span_id: String,
    #[garde(length(chars, min = 1, max = 1000))]
    pub quote: String,
    #[serde(default)]
    pub role: Role,
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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct FindingDraft {
    #[garde(length(chars, min = 3, max = 160))]
    pub title: String,
    #[garde(length(chars, min = 10, max = 4000))]
    pub description: String,
    pub check_id: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    #[garde(length(chars, max = 2000))]
    pub suggestion: String,
    #[serde(default)]
    #[garde(length(chars, max = 600))]
    pub limitation: String,
    #[garde(length(min = 1, max = MAX_EVIDENCE), dive)]
    pub evidence: Vec<Evidence>,
    // the worker names the finding it extends, or none for a new one
    #[serde(default)]
    pub existing_finding_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Finding {
    #[garde(length(chars, min = 3, max = 160))]
    pub title: String,
    #[garde(length(chars, min = 10, max = 4000))]
    pub description: String,
    pub check_id: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    #[garde(length(chars, max = 2000))]
    pub suggestion: String,
    #[serde(default)]
    #[garde(length(chars, max = 600))]
    pub limitation: String,
    #[garde(length(min = 1, max = MAX_EVIDENCE), dive)]
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
