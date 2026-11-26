use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_error_types::ProxyErrorTypes;

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
use super::team::{TeamAccessRequest, can_team_access_model};

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
    // token.config or a models list holding all-team-models skips the key check entirely
    let skip = !fields.config.0.is_empty()
        || text_models(&fields.models)
            .iter()
            .any(|name| name == ALL_TEAM_MODELS);
    if !skip {
        // the source omits the argument, and the callee's default is the proxy's own prisma_client
        can_key_call_model(model, token, context, deps.teams).await?;
    }
    if let Some(team_id) = fields.team_id.as_deref() {
        check_team_and_member(model, token, context, deps, team_id).await?;
    }
    if let Some(project_id) = fields.project_id.as_deref() {
        check_project(model, token, context, deps, project_id).await?;
    }
    Ok(())
}

// the team's own grant, the key-group override of a team denial, then the member scope on a live row
async fn check_team_and_member(
    model: &RequestedModels,
    token: &UserAPIKeyAuth,
    context: &ModelAccessContext<'_>,
    deps: &ResolvedDeps<'_>,
    team_id: &str,
) -> Result<(), EntityModelAccessError> {
    let loaded = if let Some(teams) = deps.teams {
        let request = TeamRequest {
            team_id,
            source: TeamSource::CacheThenDatabase,
            team_id_upsert: false,
        };
        get_team_object(teams, request).await.ok()
    } else {
        None
    };
    let live = loaded.is_some();
    let team = match loaded {
        Some(row) => row,
        None => fallback_team(token)?,
    };
    check_team_grant(model, token, &team.team, context, deps.teams).await?;
    if token.view.token.user_id.is_some()
        && live
        && let Some(admission) = deps.admission
    {
        check_member_scope(admission, token, &team.team, model).await?;
    }
    Ok(())
}

// the team check, then only an ownership-carrying key access group may override its denial
async fn check_team_grant(
    model: &RequestedModels,
    token: &UserAPIKeyAuth,
    team: &LiteLLMTeamTable,
    context: &ModelAccessContext<'_>,
    lookup: Option<&TeamLookup>,
) -> Result<(), EntityModelAccessError> {
    let aliases = token
        .view
        .team_model_aliases
        .as_ref()
        .map(|payload| key_model_aliases(&payload.0))
        .unwrap_or_default();
    let targets = key_model_aliases(&token.view.token.aliases.0);
    // the same default reaches the team check, so the team's own groups expand through it
    let request = TeamAccessRequest {
        team: Some(team),
        team_aliases: &aliases,
        key_aliases: &targets,
        lookup,
    };
    let checked = can_team_access_model(model, &request, context).await;
    let Err(error) = checked else {
        return Ok(());
    };
    // only team_model_access_denied can be overridden; every other exception escapes
    let team_denial = error.denial().is_some_and(|denial| {
        denial.as_proxy_exception().error_type == ProxyErrorTypes::TeamModelAccessDenied.to_string()
    });
    if !team_denial {
        return Err(error);
    }
    let grants =
        key_access_group_grants_model(model, Some(token), Some(team), lookup, context).await?;
    if grants { Ok(()) } else { Err(error) }
}

// the project check runs only when a project loaded and its own allowlist is nonempty
async fn check_project(
    model: &RequestedModels,
    token: &UserAPIKeyAuth,
    context: &ModelAccessContext<'_>,
    deps: &ResolvedDeps<'_>,
    project_id: &str,
) -> Result<(), EntityModelAccessError> {
    let Some(lookup) = deps.projects else {
        return Ok(());
    };
    let project = get_project_object(lookup, project_id, deps.now)
        .await
        .map_err(|error| EntityModelAccessError::Project(Box::new(error)))?;
    let Some(loaded) = project else {
        return Ok(());
    };
    if loaded.project.models.is_empty() {
        return Ok(());
    }
    can_project_access_model(
        model,
        Some(&loaded.project),
        &key_model_aliases(&token.view.token.aliases.0),
        context,
    )
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
