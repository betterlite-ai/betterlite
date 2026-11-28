use betterlite_mcp::types::{SessionAudience, SessionTokenKind};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) base_url: String,
    pub(super) clients: Vec<ClientSeed>,
    pub(super) servers: Vec<Value>,
    pub(super) setup: Vec<SetupStep>,
    pub(super) steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClientSeed {
    pub(super) redirect_uris: Vec<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MasterMode {
    Present,
    Absent,
    Invalid,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SetupKind {
    MintCode,
    MintToken,
    BurnRefresh,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetupStep {
    pub(super) kind: SetupKind,
    pub(super) user_id: String,
    pub(super) client: i64,
    #[serde(default)]
    pub(super) client_id: String,
    pub(super) redirect_uri: String,
    pub(super) code_challenge: String,
    pub(super) jti: String,
    pub(super) iat: i64,
    pub(super) exp: i64,
    pub(super) resource_server_id: Option<String>,
    pub(super) audience: Option<SessionAudience>,
    pub(super) team_id: Option<String>,
    pub(super) at: i64,
    pub(super) token_kind: SessionTokenKind,
    pub(super) tamper: bool,
    pub(super) bearer: bool,
    #[serde(default)]
    pub(super) token: i64,
    #[serde(default)]
    pub(super) token_text: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StepKind {
    Token,
    Revoke,
    Introspect,
    Pkce,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReloadKind {
    Unresolvable,
    Unavailable,
    Faulted,
    NoActiveKey,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MintFailureKind {
    Unresolvable,
    Unavailable,
    Faulted,
    NoActiveKey,
    NotAMember,
    TeamRequired,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RefusalKind {
    UnsupportedGrantType,
    InvalidRequest,
    TemporarilyUnavailable,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MintAnswer {
    pub(super) key: String,
    pub(super) expires_in: i64,
    pub(super) user_id: String,
    pub(super) team_id: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExchangeAnswer {
    pub(super) user_id: String,
    pub(super) team_id: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExchangeFailure {
    pub(super) error: RefusalKind,
    pub(super) description: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    pub(super) name: String,
    pub(super) kind: StepKind,
    pub(super) master: MasterMode,
    pub(super) now: i64,
    pub(super) client: i64,
    #[serde(default)]
    pub(super) client_id: String,
    pub(super) grant_type: String,
    pub(super) code: i64,
    #[serde(default)]
    pub(super) code_text: String,
    pub(super) redirect_uri: Option<String>,
    pub(super) code_verifier: String,
    pub(super) code_challenge: String,
    pub(super) refresh: i64,
    #[serde(default)]
    pub(super) refresh_text: String,
    pub(super) resource: Option<String>,
    pub(super) subject: i64,
    #[serde(default)]
    pub(super) subject_text: String,
    pub(super) subject_token_type: Option<String>,
    pub(super) requested_token_type: Option<String>,
    pub(super) token: i64,
    #[serde(default)]
    pub(super) token_text: String,
    pub(super) reload: Option<ReloadKind>,
    pub(super) mint: Option<MintAnswer>,
    pub(super) mint_failure: Option<MintFailureKind>,
    pub(super) exchange: Option<ExchangeAnswer>,
    pub(super) exchange_failure: Option<ExchangeFailure>,
}
