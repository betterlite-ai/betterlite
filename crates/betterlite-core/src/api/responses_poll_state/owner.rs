use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use serde::{Deserialize, Serialize};

// the trusted identity B080 records at admission; an empty field means it was absent
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollOwner {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub key_fingerprint: String,
}

impl PollOwner {
    #[must_use]
    pub fn from_principal(principal: &Principal, key_fingerprint: Option<&str>) -> Self {
        Self {
            user_id: principal
                .user
                .as_ref()
                .map(|user| user.id.clone())
                .unwrap_or_default(),
            team_id: principal
                .teams
                .first()
                .map(|team| team.id.clone())
                .unwrap_or_default(),
            key_fingerprint: key_fingerprint.unwrap_or_default().to_owned(),
        }
    }
}

// the caller B929's policy reads: the first team, the proxy-admin role and the key fingerprint
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PollCaller {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub platform_admin: bool,
    pub key_fingerprint: Option<String>,
}

impl PollCaller {
    #[must_use]
    pub fn from_principal(principal: &Principal, key_fingerprint: Option<&str>) -> Self {
        Self {
            user_id: principal.user.as_ref().map(|user| user.id.clone()),
            team_id: principal.teams.first().map(|team| team.id.clone()),
            platform_admin: principal.roles.contains(&Role::PlatformAdmin),
            key_fingerprint: key_fingerprint.map(str::to_owned),
        }
    }
}
