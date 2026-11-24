use betterlite_auth::auth_model_patterns::{ALL_PROXY_MODELS, ALL_TEAM_MODELS};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use secrecy::ExposeSecret as _;

use crate::key_fields::values::LimitType;
use crate::key_limit_policy::error::{Entity, KeyLimitError, Rejection};
use crate::key_limit_policy::limits::{
    MODEL_RPM_FIELD, MODEL_TPM_FIELD, check_model_limits, check_rate_limits, model_ceilings,
};
use crate::key_limit_policy::reads::{KeyLimitReads, KeyOwner};
use crate::key_limit_policy::request::KeyLimitRequest;
use crate::team_parent_limits::error::RateLimit;

const GUARANTEED: &str = "guaranteed_throughput";

pub async fn check_team_key_limits(
    team: &LiteLLMTeamTableCachedObj,
    request: &KeyLimitRequest<'_>,
    reads: &dyn KeyLimitReads,
) -> Result<(), KeyLimitError> {
    if request.rpm_limit_type() != Some(LimitType::GuaranteedThroughput)
        && request.tpm_limit_type() != Some(LimitType::GuaranteedThroughput)
    {
        return Ok(());
    }
    let base = &team.team.fields.base;
    let keys = reads.keys(KeyOwner::Team(&base.team_id)).await?;
    let keys = exclude_current(keys, request);
    check_team_model_limits(&keys, team, request)?;
    check_rate_limits(&keys, request, base.rpm_limit, base.tpm_limit, Entity::Team)?;
    Ok(())
}

pub fn check_team_model_limits(
    keys: &[LiteLLMVerificationToken],
    team: &LiteLLMTeamTableCachedObj,
    request: &KeyLimitRequest<'_>,
) -> Result<(), KeyLimitError> {
    let base = &team.team.fields.base;
    let metadata = base.metadata.as_ref();
    let rpm = model_ceilings(
        metadata,
        MODEL_RPM_FIELD,
        RateLimit::Rpm,
        Entity::Team,
        base.rpm_limit,
    )?;
    let tpm = model_ceilings(
        metadata,
        MODEL_TPM_FIELD,
        RateLimit::Tpm,
        Entity::Team,
        base.tpm_limit,
    )?;
    check_model_limits(keys, request, &rpm, &tpm)
}

pub async fn check_org_key_limits(
    organization: &OrganizationTable,
    request: &KeyLimitRequest<'_>,
    reads: &dyn KeyLimitReads,
) -> Result<(), KeyLimitError> {
    if !guaranteed(request.rpm_limit_type(), request, "rpm_limit_type")
        && !guaranteed(request.tpm_limit_type(), request, "tpm_limit_type")
    {
        return Ok(());
    }
    let organization_id = organization
        .organization_id
        .as_deref()
        .ok_or(KeyLimitError::MissingOrganization)?;
    let keys = reads.keys(KeyOwner::Organization(organization_id)).await?;
    let keys = exclude_current(keys, request);
    check_org_model_limits(&keys, organization, request)?;
    check_org_rate_limits(&keys, organization, request)?;
    Ok(())
}

pub fn check_org_model_limits(
    keys: &[LiteLLMVerificationToken],
    organization: &OrganizationTable,
    request: &KeyLimitRequest<'_>,
) -> Result<(), KeyLimitError> {
    let metadata = organization.metadata.as_ref().map(|payload| &payload.0);
    let budget = organization.litellm_budget_table.as_ref();
    let rpm = model_ceilings(
        metadata,
        MODEL_RPM_FIELD,
        RateLimit::Rpm,
        Entity::Organization,
        budget.and_then(|table| table.rpm_limit),
    )?;
    let tpm = model_ceilings(
        metadata,
        MODEL_TPM_FIELD,
        RateLimit::Tpm,
        Entity::Organization,
        budget.and_then(|table| table.tpm_limit),
    )?;
    check_model_limits(keys, request, &rpm, &tpm)
}

pub fn check_org_rate_limits(
    keys: &[LiteLLMVerificationToken],
    organization: &OrganizationTable,
    request: &KeyLimitRequest<'_>,
) -> Result<(), Rejection> {
    let budget = organization.litellm_budget_table.as_ref();
    check_rate_limits(
        keys,
        request,
        budget.and_then(|table| table.rpm_limit),
        budget.and_then(|table| table.tpm_limit),
        Entity::Organization,
    )
}

pub async fn check_project_key_limits(
    project_id: &str,
    request: &KeyLimitRequest<'_>,
    reads: &dyn KeyLimitReads,
) -> Result<(), KeyLimitError> {
    let project = reads.project(project_id).await?;
    let Some(project) = project else {
        return Err(KeyLimitError::from(Rejection::ProjectNotFound {
            project_id: project_id.to_owned(),
        }));
    };
    check_project_models(project_id, &project, request)?;
    check_project_budget(project_id, &project, request)
}

pub async fn validate_org_assignment(
    caller_user_id: Option<&str>,
    organization_id: &str,
    reads: &dyn KeyLimitReads,
) -> Result<(), KeyLimitError> {
    let Some(user_id) = caller_user_id else {
        return Err(KeyLimitError::from(Rejection::AssignWithoutUser));
    };
    let memberships = reads.member_organizations(user_id).await?;
    if !memberships.contains(organization_id) {
        return Err(KeyLimitError::from(Rejection::NotMember {
            organization_id: organization_id.to_owned(),
        }));
    }
    Ok(())
}

// data.key may be a raw sk-... key or an already hashed token; both exclude the key being updated
fn exclude_current(
    keys: Vec<LiteLLMVerificationToken>,
    request: &KeyLimitRequest<'_>,
) -> Vec<LiteLLMVerificationToken> {
    let Some(current) = request.update_key_hash() else {
        return keys;
    };
    keys.into_iter()
        .filter(|key| {
            key.token.as_ref().map(|token| token.expose_secret()) != Some(current.as_str())
        })
        .collect()
}

// an organization's type field falls back to its metadata key; a team's does not
fn guaranteed(field: Option<LimitType>, request: &KeyLimitRequest<'_>, key: &str) -> bool {
    field.map_or_else(
        || {
            matches!(
                request.metadata().and_then(|metadata| metadata.get(key)),
                Some(JsonData::String(text)) if text == GUARANTEED
            )
        },
        |kind| kind == LimitType::GuaranteedThroughput,
    )
}

fn check_project_models(
    project_id: &str,
    project: &CachedProject,
    request: &KeyLimitRequest<'_>,
) -> Result<(), KeyLimitError> {
    let allowed = &project.project.models;
    let Some(models) = request.models().filter(|_| !allowed.is_empty()) else {
        return Ok(());
    };
    for model in models {
        if !permitted(model, allowed) {
            return Err(KeyLimitError::from(Rejection::ProjectModel {
                model: model.clone(),
                allowed: allowed.clone(),
                project_id: project_id.to_owned(),
            }));
        }
    }
    Ok(())
}

fn permitted(model: &JsonData, allowed: &[String]) -> bool {
    let JsonData::String(name) = model else {
        return false;
    };
    allowed.contains(name) || name == ALL_TEAM_MODELS || name == ALL_PROXY_MODELS
}

fn check_project_budget(
    project_id: &str,
    project: &CachedProject,
    request: &KeyLimitRequest<'_>,
) -> Result<(), KeyLimitError> {
    let ceiling = project
        .project
        .litellm_budget_table
        .as_ref()
        .and_then(|table| table.max_budget);
    let (Some(requested), Some(ceiling)) = (request.max_budget(), ceiling) else {
        return Ok(());
    };
    if requested > ceiling {
        return Err(KeyLimitError::from(Rejection::ProjectBudget {
            requested,
            ceiling,
            project_id: project_id.to_owned(),
        }));
    }
    Ok(())
}
