use betterlite_auth::user_roles::UserRole;

// The caller facts these checks read: the role, whether the credential is a CLI session token, and the UI team-key flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub role: Option<UserRole>,
    pub is_session_token: bool,
    pub is_ui_session_team_key: bool,
}
