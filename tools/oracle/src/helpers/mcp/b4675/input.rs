use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) clients: Vec<ClientSeed>,
    pub(super) servers: Vec<Value>,
    pub(super) steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClientSeed {
    pub(super) redirect_uris: Vec<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Aggregate,
    Native,
    Describe,
    Complete,
    Contract,
}

#[derive(Clone, Copy)]
pub(super) enum ReplyKind {
    Aggregate,
    Native,
    Describe,
    Complete,
}

impl Kind {
    pub(super) const fn reply_kind(self) -> Option<ReplyKind> {
        match self {
            Self::Aggregate => Some(ReplyKind::Aggregate),
            Self::Native => Some(ReplyKind::Native),
            Self::Describe => Some(ReplyKind::Describe),
            Self::Complete => Some(ReplyKind::Complete),
            Self::Contract => None,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VendorKind {
    Present,
    Absent,
    Unavailable,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TeamsFailure {
    Unresolvable,
    Unavailable,
    Faulted,
    NoActiveKey,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TeamSeed {
    pub(super) team_id: String,
    pub(super) team_alias: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    pub(super) name: String,
    pub(super) kind: Kind,
    pub(super) now: i64,
    pub(super) path: String,
    pub(super) query: Option<String>,
    pub(super) cookie_from: i64,
    pub(super) client: i64,
    pub(super) client_id: String,
    pub(super) redirect_uri: String,
    pub(super) state: String,
    pub(super) code_challenge: Option<String>,
    pub(super) code_challenge_method: Option<String>,
    pub(super) response_type: Option<String>,
    pub(super) user: Option<String>,
    pub(super) resource: Option<String>,
    pub(super) tokens: Vec<String>,
    pub(super) handle: String,
    pub(super) delivery: Option<String>,
    pub(super) team_id: Option<String>,
    pub(super) decision: Option<String>,
    pub(super) teams: Vec<TeamSeed>,
    pub(super) teams_failure: Option<TeamsFailure>,
    pub(super) vendor: VendorKind,
    pub(super) reachable: bool,
    pub(super) token_exchange: bool,
}
