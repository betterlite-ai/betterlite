use betterlite_db::identity_rows::key::LiteLLMVerificationToken;

use super::rows::RowKind;
use crate::auth_cache::keys::CacheKey;

// litellm/proxy/auth/auth_object_prefetch.py: _TEAM_MEMBERSHIP_AUTH_TTL
pub const MEMBERSHIP_AUTH_TTL: f64 = 5.0;
// LiteLLM reads every id from the token; None means the request does not reference that object
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
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
        Self {
            user_id: token.user_id.clone(),
            team_id: token.team_id.clone(),
            membership_user_id: token.team_id.as_ref().and_then(|_| token.user_id.clone()),
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
    let mut plan = Vec::with_capacity(7);
    if let Some(user_id) = &refs.user_id {
        plan.push((user_id.clone(), RowKind::User, Some(management_ttl)));
    }
    if let Some(team_id) = &refs.team_id {
        plan.push((
            format!("team_id:{team_id}"),
            RowKind::Team,
            Some(management_ttl),
        ));
    }
    if let (Some(team_id), Some(user_id)) = (&refs.team_id, &refs.membership_user_id) {
        let auth_key = CacheKey::TeamMembershipAuth { team_id, user_id }.render();
        plan.push((auth_key, RowKind::Membership, Some(MEMBERSHIP_AUTH_TTL)));
        let reservation_key = CacheKey::TeamMembershipReservation { user_id, team_id }.render();
        plan.push((reservation_key, RowKind::Membership, None));
    }
    if let Some(organization_id) = &refs.organization_id {
        let key = format!("org_id:{organization_id}");
        plan.push((key, RowKind::Organization, Some(organization_ttl)));
        let budget_key = format!("org_id:{organization_id}:with_budget");
        plan.push((budget_key, RowKind::Organization, Some(organization_ttl)));
    }
    if let Some(project_id) = &refs.project_id {
        plan.push((
            CacheKey::ProjectId(project_id).render(),
            RowKind::Project,
            Some(management_ttl),
        ));
    }
    plan.into_iter()
        .map(|(key, row, ttl)| PrefetchEntry { key, row, ttl })
        .collect()
}
