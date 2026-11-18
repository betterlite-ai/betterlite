use betterlite_auth::user_roles::UserRole;
use serde::Deserialize;

// The caller facts these checks read: the role, whether the credential is a CLI session token, and the UI team-key flag.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Caller {
    pub role: Option<UserRole>,
    pub is_session_token: bool,
    pub is_ui_session_team_key: bool,
}
