use betterlite_db::identity_rows::key::LiteLLMVerificationToken;

use crate::auth_cache::keys::CacheKey;

use super::rows::RowKind;

// litellm/proxy/auth/auth_object_prefetch.py: _TEAM_MEMBERSHIP_AUTH_TTL
pub const MEMBERSHIP_AUTH_TTL: f64 = 5.0;

// LiteLLM reads every id from the token; None means the request does not reference that object
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthObjectRefs {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub membership_user_id: Option<String>,
    pub organization_id: Option<String>,
    pub project_id: Option<String>,
}

impl AuthObjectRefs {
    // a membership needs both ids, and an empty string is still an id
    #[must_use]
    pub fn from_token(token: &LiteLLMVerificationToken) -> Self {
        let membership_user_id = match (&token.team_id, &token.user_id) {
            (Some(_team), Some(user)) => Some(user.clone()),
            (Some(_team), None) | (None, Some(_user)) | (None, None) => None,
        };
        Self {
            user_id: token.user_id.clone(),
            team_id: token.team_id.clone(),
            membership_user_id,
            organization_id: token.org_id.clone(),
            project_id: token.project_id.clone(),
        }
    }
}

// one cache entry the request will read, with the model that row validates as
#[derive(Clone, Debug, PartialEq)]
pub struct PrefetchEntry {
    pub key: String,
    pub row: RowKind,
    pub ttl: Option<f64>,
}

// the source yields the seven entries in this order and never merges equal keys
#[must_use]
pub fn iter_entries(
    refs: &AuthObjectRefs,
    management_ttl: f64,
    organization_ttl: f64,
) -> Vec<PrefetchEntry> {
    let mut entries = Vec::with_capacity(7);
    if let Some(user_id) = &refs.user_id {
        entries.push(PrefetchEntry {
            key: user_id.clone(),
            row: RowKind::User,
            ttl: Some(management_ttl),
        });
    }
    if let Some(team_id) = &refs.team_id {
        entries.push(PrefetchEntry {
            key: format!("team_id:{team_id}"),
            row: RowKind::Team,
            ttl: Some(management_ttl),
        });
    }
    if let (Some(team_id), Some(user_id)) = (&refs.team_id, &refs.membership_user_id) {
        entries.push(PrefetchEntry {
            key: CacheKey::TeamMembershipAuth { team_id, user_id }.render(),
            row: RowKind::Membership,
            ttl: Some(MEMBERSHIP_AUTH_TTL),
        });
        entries.push(PrefetchEntry {
            key: CacheKey::TeamMembershipReservation { user_id, team_id }.render(),
            row: RowKind::Membership,
            ttl: None,
        });
    }
    if let Some(organization_id) = &refs.organization_id {
        entries.push(PrefetchEntry {
            key: format!("org_id:{organization_id}"),
            row: RowKind::Organization,
            ttl: Some(organization_ttl),
        });
        entries.push(PrefetchEntry {
            key: format!("org_id:{organization_id}:with_budget"),
            row: RowKind::Organization,
            ttl: Some(organization_ttl),
        });
    }
    if let Some(project_id) = &refs.project_id {
        entries.push(PrefetchEntry {
            key: CacheKey::ProjectId(project_id).render(),
            row: RowKind::Project,
            ttl: Some(management_ttl),
        });
    }
    entries
}
