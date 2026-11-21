use betterlite_auth::user_roles::UserRole;
use serde::{Deserialize, Serialize};

// the UserAPIKeyAuth fields every memory helper reads, built into a real key so the production
// principal conversion runs rather than a stand-in
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallerInput {
    user_id: Option<String>,
    user_role: Option<UserRole>,
    team_id: Option<String>,
    metadata: CallerMetadata,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallerMetadata {
    service_account_id: Option<String>,
}

// the memory row fields _assert_write_access reads; the rest of the row is never consulted
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RowInput {
    pub(super) user_id: Option<String>,
    pub(super) team_id: Option<String>,
}

// _internal_error's answer: the status and the generic detail FastAPI would send
#[derive(Debug, Serialize)]
pub(super) struct InternalErrorValue {
    pub(super) status: u16,
    pub(super) detail: String,
}

// the exception _internal_error logs, carrying the message the case gives it
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct CaseError {
    pub(super) message: String,
}
