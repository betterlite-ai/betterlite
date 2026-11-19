use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) runs: Vec<Run>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Run {
    pub(super) model: String,
    pub(super) pending: Option<String>,
    pub(super) frames: Vec<String>,
    pub(super) chunks: Vec<String>,
    pub(super) block: Block,
    pub(super) receive_failure: Option<usize>,
    pub(super) send_failure: Option<usize>,
    pub(super) open_error: bool,
    pub(super) client_send_failure: Option<usize>,
    pub(super) credentials: CredentialMode,
    pub(super) auth: Option<Auth>,
    pub(super) api_base: Option<String>,
    pub(super) endpoint: Option<String>,
    pub(super) region: Option<String>,
    pub(super) committed: bool,
    pub(super) failure: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Auth {
    pub(super) access_key_id: String,
    pub(super) secret_access_key: String,
    pub(super) session_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CredentialMode {
    Resolved,
    Absent,
    Error,
}

// One direction stays pending so the other is always the task that finishes first
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Block {
    None,
    Client,
    Stream,
}
