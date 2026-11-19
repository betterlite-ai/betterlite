use betterlite_config::sections::settings::JsonValue;

const PRE_CALL: &str = "pre_call";
const NOT_RUN: &str = "not_run";
const SUCCESS: &str = "success";
const INTERVENED: &str = "guardrail_intervened";
const FAILED: &str = "failed";
const BLOCKED: &str = "blocked";

// LiteLLM drops an entry whose guardrail_status is exactly "not_run"; every other value counts as applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardrailStatus {
    NotRun,
    Success,
    Intervened,
    Failed,
    Blocked,
    Other,
}

impl GuardrailStatus {
    #[must_use]
    pub fn from_value(value: Option<&JsonValue>) -> Self {
        let Some(JsonValue::String(text)) = value else {
            return Self::Other;
        };
        match text.as_str() {
            NOT_RUN => Self::NotRun,
            SUCCESS => Self::Success,
            INTERVENED => Self::Intervened,
            FAILED => Self::Failed,
            BLOCKED => Self::Blocked,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardrailEvidence {
    pub status: GuardrailStatus,
    pub mode: GuardrailMode,
}

// guardrail_mode is logged as the configured mode, so it has the shapes LitellmParams.mode can take.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuardrailMode {
    Absent,
    Text(String),
    List(Vec<ModeBranch>),
    Mapping(ModeMapping),
    Unsupported,
}

impl GuardrailMode {
    #[must_use]
    pub fn from_value(value: Option<&JsonValue>) -> Self {
        match value {
            None | Some(JsonValue::Null) => Self::Absent,
            Some(JsonValue::String(text)) => Self::Text(text.clone()),
            Some(JsonValue::Array(items)) => {
                Self::List(items.iter().map(ModeBranch::from_value).collect())
            }
            Some(JsonValue::Object(fields)) => Self::Mapping(ModeMapping {
                default: fields
                    .get("default")
                    .filter(|branch| !branch.is_null())
                    .map(ModeBranch::from_value),
                tags: fields
                    .get("tags")
                    .and_then(tag_branches)
                    .unwrap_or_default(),
            }),
            Some(JsonValue::Bool(_) | JsonValue::Number(_)) => Self::Unsupported,
        }
    }

    // True only when the logged mode guarantees the guardrail ran in `mode` for the audited request.
    #[must_use]
    pub fn matches(&self, mode: &str) -> bool {
        match self {
            Self::Absent => mode == PRE_CALL,
            Self::Text(text) => text == mode,
            Self::List(branches) => list_runs_in(branches, mode),
            Self::Mapping(mapping) => mapping.matches(mode),
            Self::Unsupported => false,
        }
    }
}

// A Mode object's branches: its non-null default and the values of a dictionary-valued tags field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModeMapping {
    pub default: Option<ModeBranch>,
    pub tags: Vec<ModeBranch>,
}

impl ModeMapping {
    fn matches(&self, mode: &str) -> bool {
        let Some(default) = self.default.as_ref() else {
            return false;
        };
        default.runs_in(mode) && self.tags.iter().all(|tag| tag.runs_in(mode))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModeBranch {
    Text(String),
    List(Vec<ModeBranch>),
    Other,
}

impl ModeBranch {
    #[must_use]
    pub fn from_value(value: &JsonValue) -> Self {
        match (value.as_str(), value.as_array()) {
            (Some(text), _) => Self::Text(text.to_owned()),
            (None, Some(items)) => Self::List(items.iter().map(Self::from_value).collect()),
            (None, None) => Self::Other,
        }
    }

    fn runs_in(&self, mode: &str) -> bool {
        match self {
            Self::Text(text) => text == mode,
            Self::List(branches) => list_runs_in(branches, mode),
            Self::Other => false,
        }
    }

    // A list element equals a mode string only when the element itself is that string, as in Python.
    fn equals(&self, mode: &str) -> bool {
        match self {
            Self::Text(text) => text == mode,
            Self::List(_) | Self::Other => false,
        }
    }
}

fn tag_branches(tags: &JsonValue) -> Option<Vec<ModeBranch>> {
    let JsonValue::Object(branches) = tags else {
        return None;
    };
    Some(branches.values().map(ModeBranch::from_value).collect())
}

fn list_runs_in(branches: &[ModeBranch], mode: &str) -> bool {
    if branches.is_empty() {
        return false;
    }
    branches.iter().all(|branch| branch.equals(mode))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComplianceRequest {
    pub request_id: String,
    pub user_id: Option<String>,
    pub model: Option<String>,
    pub timestamp: Option<String>,
    pub guardrail_information: Vec<GuardrailEvidence>,
}
