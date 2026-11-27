use crate::admitted_user::DelegatedHuman;
use crate::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use crate::ui_session_identity::{
    UiSessionTeams, is_ui_session_credential, resolve_ui_session_team_ids,
};

mod tests;

// What the caller's own credential grants before a team or admitted-user context widens it.
#[derive(Clone, Copy, Debug)]
pub struct OwnGrantView<'auth> {
    pub has_object_permission: bool,
    pub object_permission_id: Option<&'auth str>,
    pub has_admin_view: bool,
}

#[must_use]
pub fn with_team_id(auth: &UserAPIKeyAuth, team_id: &str) -> UserAPIKeyAuth {
    let mut cloned = auth.clone();
    cloned.view.token.team_id = Some(team_id.to_owned());
    cloned
}

#[must_use]
pub fn with_parent_span_of(auth: &UserAPIKeyAuth, session: &UserAPIKeyAuth) -> UserAPIKeyAuth {
    let mut copied = auth.clone();
    copied
        .extra
        .parent_otel_span
        .clone_from(&session.extra.parent_otel_span);
    copied
}

#[must_use]
pub fn own_grant_view(auth: &UserAPIKeyAuth) -> OwnGrantView<'_> {
    OwnGrantView {
        has_object_permission: auth.view.token.object_permission.is_some(),
        object_permission_id: auth.view.token.object_permission_id.as_deref(),
        has_admin_view: user_api_key_has_admin_view(auth),
    }
}

pub async fn admitted_user_context<H>(auth: &UserAPIKeyAuth, human: &H) -> Option<UserAPIKeyAuth>
where
    H: DelegatedHuman,
{
    if !is_ui_session_credential(auth) {
        return None;
    }
    let Some(user_id) = auth.view.token.user_id.as_deref() else {
        return None;
    };
    match human
        .reload_admitted_user(user_id, auth.extra.requires_fresh_policy)
        .await
    {
        Ok(admitted) => Some(with_parent_span_of(&admitted, auth)),
        Err(error) => {
            // LiteLLM warns and drops the user context on every reload failure a caller can see
            tracing::warn!(
                "MCP dashboard session: admitted-subject reload failed for {user_id}: {error}"
            );
            None
        }
    }
}

pub async fn build_effective_auth_contexts<U, H>(
    auth: &UserAPIKeyAuth,
    teams: &U,
    human: &H,
) -> Vec<UserAPIKeyAuth>
where
    U: UiSessionTeams + ?Sized,
    H: DelegatedHuman,
{
    let team_ids = resolve_ui_session_team_ids(auth, teams).await;
    let mut contexts = Vec::new();
    if team_ids.is_empty() {
        contexts.push(auth.clone());
    } else {
        for team_id in &team_ids {
            contexts.push(with_team_id(auth, team_id));
        }
    }
    if let Some(admitted) = admitted_user_context(auth, human).await {
        contexts.push(admitted);
    }
    contexts
}
