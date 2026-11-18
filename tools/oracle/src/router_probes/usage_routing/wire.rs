use serde::{Deserialize, Serialize};

fn default_group() -> String {
    "probe-group".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default = "default_group")]
    pub group: String,
    #[serde(default)]
    pub ttl: Option<i64>,
    pub deployments: Vec<Entry>,
    pub steps: Vec<Step>,
}

// A deployment as get_available_deployments reads it: the id, and one tpm and rpm limit at each level the
// pinned fallback walks, so a case can say which slot a limit sits in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    #[serde(default)]
    pub tpm: Option<i64>,
    #[serde(default)]
    pub params_tpm: Option<i64>,
    #[serde(default)]
    pub info_tpm: Option<i64>,
    #[serde(default)]
    pub rpm: Option<i64>,
    #[serde(default)]
    pub params_rpm: Option<i64>,
    #[serde(default)]
    pub info_rpm: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Success,
    AsyncSuccess,
    Select,
    Observe,
    Gate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    #[serde(default)]
    pub now_millis: i64,
    #[serde(default)]
    pub deployment_id: Option<String>,
    #[serde(default)]
    pub total_tokens: Option<i64>,
    // absent metadata is what a real success event always carries for a routed call
    #[serde(default)]
    pub metadata: Option<bool>,
    #[serde(default)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub token_count_failure: Option<bool>,
    // the deployments a select offers; empty means every deployment, in the case order
    #[serde(default)]
    pub healthy: Vec<String>,
    #[serde(default)]
    pub fail_reads: Option<bool>,
    #[serde(default)]
    pub fail_writes: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Observation {
    Recorded {
        tpm: Option<Vec<(String, i64)>>,
        rpm: Option<Vec<(String, i64)>>,
        tpm_ttl: Option<i64>,
        rpm_ttl: Option<i64>,
    },
    Selected {
        id: Option<String>,
    },
    Gated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<Observation>,
}
