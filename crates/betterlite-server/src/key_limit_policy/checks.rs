use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::{ALL_PROXY_MODELS, ALL_TEAM_MODELS};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

use crate::key_fields::values::LimitType;
use crate::key_limit_policy::error::{Entity, KeyLimitError, Rejection};
use crate::key_limit_policy::limits::{
    MODEL_RPM_FIELD, MODEL_TPM_FIELD, check_model_limits, check_rate_limits, model_ceilings,
};
use crate::key_limit_policy::reads::{KeyLimitReads, KeyOwner};
use crate::key_limit_policy::request::KeyLimitRequest;
use crate::team_parent_limits::error::RateLimit;

const GUARANTEED: &str = "guaranteed_throughput";

// one entity's aggregate RPM/TPM ceilings and the metadata holding its per-model maps
struct EntityLimits<'entity> {
    entity: Entity,
    metadata: Option<&'entity BTreeMap<String, JsonData>>,
    rpm: Option<i64>,
    tpm: Option<i64>,
}

pub async fn check_team_key_limits(
    team: &LiteLLMTeamTableCachedObj,
    request: &KeyLimitRequest,
    reads: &dyn KeyLimitReads,
) -> Result<(), KeyLimitError> {
    if request.rpm_limit_type != Some(LimitType::GuaranteedThroughput)
        && request.tpm_limit_type != Some(LimitType::GuaranteedThroughput)
    {
        return Ok(());
    }
    let base = &team.team.fields.base;
    let limits = EntityLimits {
        entity: Entity::Team,
        metadata: base.metadata.as_ref(),
        rpm: base.rpm_limit,
        tpm: base.tpm_limit,
    };
    check_entity(request, reads, KeyOwner::Team(&base.team_id), &limits).await
}

pub async fn check_org_key_limits(
    organization: &OrganizationTable,
    request: &KeyLimitRequest,
    reads: &dyn KeyLimitReads,
) -> Result<(), KeyLimitError> {
    if !guaranteed(request.rpm_limit_type, request, "rpm_limit_type")
        && !guaranteed(request.tpm_limit_type, request, "tpm_limit_type")
    {
        return Ok(());
    }
    let organization_id = organization
        .organization_id
        .as_deref()
        .ok_or(KeyLimitError::MissingOrganization)?;
    let budget = organization.litellm_budget_table.as_ref();
    let limits = EntityLimits {
        entity: Entity::Organization,
        metadata: organization.metadata.as_ref().map(|payload| &payload.0),
        rpm: budget.and_then(|table| table.rpm_limit),
        tpm: budget.and_then(|table| table.tpm_limit),
    };
    check_entity(
        request,
        reads,
        KeyOwner::Organization(organization_id),
        &limits,
    )
    .await
}

// data.key may be raw or hashed; either way that key's own limits are not counted, then model limits run first
async fn check_entity(
    request: &KeyLimitRequest,
    reads: &dyn KeyLimitReads,
    owner: KeyOwner<'_>,
    limits: &EntityLimits<'_>,
) -> Result<(), KeyLimitError> {
    let mut keys = reads.keys(owner).await?;
    if let Some(current) = request.update_key_hash.as_deref() {
        keys.retain(|key| {
            key.token.as_ref().map(secrecy::ExposeSecret::expose_secret) != Some(current)
        });
    }
    let (entity, metadata) = (limits.entity, limits.metadata);
    let rpm = model_ceilings(
        metadata,
        MODEL_RPM_FIELD,
        RateLimit::Rpm,
        entity,
        limits.rpm,
    )?;
    let tpm = model_ceilings(
        metadata,
        MODEL_TPM_FIELD,
        RateLimit::Tpm,
        entity,
        limits.tpm,
    )?;
    check_model_limits(&keys, request, &rpm, &tpm)?;
    check_rate_limits(&keys, request, limits.rpm, limits.tpm, entity)?;
    Ok(())
}

pub async fn check_project_key_limits(
    project_id: &str,
    request: &KeyLimitRequest,
    reads: &dyn KeyLimitReads,
) -> Result<(), KeyLimitError> {
    let stored = reads.project(project_id).await?;
    let Some(project) = stored else {
        return Err(KeyLimitError::from(Rejection::ProjectNotFound {
            project_id: project_id.to_owned(),
        }));
    };
    let allowed = &project.project.models;
    if let Some(models) = request.models.as_deref().filter(|_| !allowed.is_empty()) {
        for model in models {
            if !permitted(model, allowed) {
                return Err(KeyLimitError::from(Rejection::ProjectModel {
                    model: model.clone(),
                    allowed: allowed.clone(),
                    project_id: project_id.to_owned(),
                }));
            }
        }
    }
    let cap = project
        .project
        .litellm_budget_table
        .as_ref()
        .and_then(|table| table.max_budget);
    let (Some(requested), Some(ceiling)) = (request.max_budget, cap) else {
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

// an organization's type field falls back to its metadata key; a team's does not
fn guaranteed(field: Option<LimitType>, request: &KeyLimitRequest, key: &str) -> bool {
    field.map_or_else(
        || {
            matches!(
                request.metadata.as_ref().and_then(|metadata| metadata.get(key)),
                Some(JsonData::String(text)) if text == GUARANTEED
            )
        },
        |kind| kind == LimitType::GuaranteedThroughput,
    )
}

fn permitted(model: &JsonData, allowed: &[String]) -> bool {
    let JsonData::String(name) = model else {
        return false;
    };
    allowed.contains(name) || name == ALL_TEAM_MODELS || name == ALL_PROXY_MODELS
}
