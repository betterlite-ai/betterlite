use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use tracing::debug;

use crate::auth_prefetch::rows::CachedProject;
use crate::model_grant_intersection::collect::{MatchedRequest, collect_matched_groups};
use crate::model_grant_intersection::sources::{GrantSources, ServingGroups};
use crate::model_grant_intersection::token::GrantToken;
use crate::resolved_key::auth::UserAPIKeyAuth;

// stamp_matched_model_access_groups' inputs: the collection inputs plus the auth object to stamp.
pub struct StampRequest<'request> {
    pub models: Option<&'request [String]>,
    pub auth: Option<&'request mut UserAPIKeyAuth>,
    pub team: Option<&'request LiteLLMTeamTableCachedObj>,
    pub project: Option<&'request CachedProject>,
    pub membership: Option<LiteLLMTeamMembership>,
    pub membership_loaded: bool,
    pub groups: Option<&'request dyn ServingGroups>,
    pub sources: Option<&'request dyn GrantSources>,
}

// stamp_matched_model_access_groups: best-effort attribution; a failure leaves any existing stamp untouched.
pub async fn stamp_matched_groups(request: StampRequest<'_>) -> Vec<String> {
    let StampRequest {
        models,
        auth: auth_option,
        team,
        project,
        membership,
        membership_loaded,
        groups,
        sources,
    } = request;
    let Some(auth) = auth_option else {
        return Vec::new();
    };
    let token = GrantToken::of(auth);
    let collected = collect_matched_groups(MatchedRequest {
        models,
        token: Some(&token),
        team,
        project,
        membership,
        membership_loaded,
        groups,
        sources,
    })
    .await;
    let matched = match collected {
        Ok(matched) => matched,
        Err(error) => {
            debug!("model access group attribution failed: {error}");
            return Vec::new();
        }
    };
    if matched.is_empty() {
        return Vec::new();
    }
    auth.extra.matched_model_access_groups = Some(matched.clone());
    matched
}

// The stamp the spend writer and the reservation counters read, as the auth object carries it.
#[must_use]
pub fn matched_model_access_groups(auth: &UserAPIKeyAuth) -> &[String] {
    match &auth.extra.matched_model_access_groups {
        Some(groups) => groups,
        None => &[],
    }
}
