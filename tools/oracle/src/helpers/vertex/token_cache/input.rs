use betterlite_vertex::token_cache::state::TokenState;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    #[serde(default)]
    pub(crate) credentials: Vec<CredentialCase>,
    #[serde(default)]
    pub(crate) records: Vec<RecordCase>,
    #[serde(default)]
    pub(crate) probes: Vec<ProbeCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialCase {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) token: Option<String>,
    #[serde(default)]
    pub(crate) state: Option<StateName>,
    #[serde(default)]
    pub(crate) expired: Option<bool>,
    #[serde(default)]
    pub(crate) valid: Option<bool>,
    #[serde(default)]
    pub(crate) quota_project_id: Option<String>,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordCase {
    pub(crate) credential: String,
    #[serde(default)]
    pub(crate) credentials: Option<String>,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) cached_project: Option<String>,
    #[serde(default)]
    pub(crate) legacy: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeCase {
    #[serde(default)]
    pub(crate) credentials: Option<String>,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) requested_project: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StateName {
    Fresh,
    Stale,
    Invalid,
}

impl StateName {
    pub(crate) const fn state(self) -> TokenState {
        match self {
            Self::Fresh => TokenState::Fresh,
            Self::Stale => TokenState::Stale,
            Self::Invalid => TokenState::Invalid,
        }
    }
}
