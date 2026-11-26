use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::auth_model_patterns::ALL_TEAM_MODELS;
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::grant::key_model_aliases;
use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_org_lookup::project::get_project_object;
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::lookup::{TeamLookup, TeamRequest, TeamSource};
use crate::auth_team_lookup::team::get_team_object;
use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::team_budget_admission::member_scope::check_team_member_model_access;
use crate::team_budget_admission::sources::Membership;
use crate::team_budget_admission::{TeamAdmission, TeamRequest as MemberRequest};

use super::error::EntityModelAccessError;
use super::grant::{fallback_team, text_models};
use super::groups::key_access_group_grants_model;
use super::key::can_key_call_model;
use super::project::can_project_access_model;
use super::team::can_team_access_model;

// can_key_call_resolved_model's collaborators: the two entity reads, the member scope and the clock
pub struct ResolvedDeps<'deps> {
    pub teams: Option<&'deps TeamLookup>,
    pub projects: Option<&'deps OrgLookup>,
    // the member scope runs when the caller owns it; the pipeline's budget stage passes it elsewhere
    pub admission: Option<&'deps TeamAdmission<'deps>>,
    pub now: f64,
}

// can_key_call_resolved_model: key, team, member scope and project in the source's order
pub async fn can_key_call_resolved_model(
    model: &RequestedModels,
    token: &UserAPIKeyAuth,
    context: &ModelAccessContext<'_>,
    deps: &ResolvedDeps<'_>,
) -> Result<(), EntityModelAccessError> {
    let fields = &token.view.token;
    let skip_key_check = !fields.config.0.is_empty()
        || text_models(&fields.models)
            .iter()
            .any(|name| name == ALL_TEAM_MODELS);
    if !skip_key_check {
        // the source omits the argument, and the callee's default is the proxy's own prisma_client
        can_key_call_model(model, token, context, deps.teams).await?;
    }
    let mut team_object: Option<LiteLLMTeamTableCachedObj> = None;
    let mut from_lookup = false;
    if let Some(team_id) = fields.team_id.as_deref() {
        let mut found = None;
        if let Some(lookup) = deps.teams {
            let request = TeamRequest {
                team_id,
                source: TeamSource::CacheThenDatabase,
                team_id_upsert: false,
            };
            found = get_team_object(lookup, request).await.ok();
        }
        if let Some(loaded) = found {
            team_object = Some(loaded);
            from_lookup = true;
        } else {
            team_object = Some(fallback_team(token)?);
        }
    }
    if let Some(loaded) = &team_object {
        let aliases = token
            .view
            .team_model_aliases
            .as_ref()
            .map(|payload| key_model_aliases(&payload.0))
            .unwrap_or_default();
        let targets = key_model_aliases(&token.view.token.aliases.0);
        // the same default reaches the team check, so the team's own groups expand through it
        let checked = can_team_access_model(
            model,
            Some(&loaded.team),
            &aliases,
            &targets,
            context,
            deps.teams,
        )
        .await;
        if let Err(error) = checked {
            // only team_model_access_denied can be overridden; every other exception escapes
            if !error.is_team_denial() {
                return Err(error);
            }
            let grants = key_access_group_grants_model(
                model,
                Some(token),
                Some(&loaded.team),
                deps.teams,
                context,
            )
            .await?;
            if !grants {
                return Err(error);
            }
        }
        // the member scope needs a live team; the reconstructed fallback never answers it
        if fields.user_id.is_some()
            && from_lookup
            && let Some(admission) = deps.admission
        {
            check_member_scope(admission, token, &loaded.team, model).await?;
        }
    }
    if let Some(project_id) = fields.project_id.as_deref() {
        let mut project = None;
        if let Some(lookup) = deps.projects {
            project = get_project_object(lookup, project_id, deps.now)
                .await
                .map_err(|error| EntityModelAccessError::Project(Box::new(error)))?;
        }
        if let Some(loaded) = &project
            && !loaded.project.models.is_empty()
        {
            can_project_access_model(
                model,
                Some(&loaded.project),
                &key_model_aliases(&token.view.token.aliases.0),
                context,
            )?;
        }
    }
    Ok(())
}

// _check_team_member_model_access: a loaded membership is what the scope reads, and the key aliases ride along
async fn check_member_scope(
    admission: &TeamAdmission<'_>,
    token: &UserAPIKeyAuth,
    team: &LiteLLMTeamTable,
    model: &RequestedModels,
) -> Result<(), EntityModelAccessError> {
    let request = MemberRequest {
        team: Some(team),
        token: Some(token),
        user_email: None,
        model,
        membership: Membership::Unloaded,
    };
    check_team_member_model_access(admission, &request)
        .await
        .map_err(EntityModelAccessError::from_member)
}
