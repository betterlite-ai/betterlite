use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::{ALL_PROXY_MODELS, ALL_TEAM_MODELS};
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
    let metadata = base.metadata.as_ref();
    model_limits(
        &keys,
        request,
        metadata,
        Entity::Team,
        base.rpm_limit,
        base.tpm_limit,
    )?;
    check_rate_limits(&keys, request, base.rpm_limit, base.tpm_limit, Entity::Team)
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
    let metadata = organization.metadata.as_ref().map(|payload| &payload.0);
    let budget = organization.litellm_budget_table.as_ref();
    let rpm_limit = budget.and_then(|table| table.rpm_limit);
    let tpm_limit = budget.and_then(|table| table.tpm_limit);
    model_limits(
        &keys,
        request,
        metadata,
        Entity::Organization,
        rpm_limit,
        tpm_limit,
    )?;
    check_rate_limits(&keys, request, rpm_limit, tpm_limit, Entity::Organization)?;
    Ok(())
}

// the two model-ceiling checks both entities share: aggregate ceiling and metadata per-model map
fn model_limits(
    keys: &[LiteLLMVerificationToken],
    request: &KeyLimitRequest<'_>,
    metadata: Option<&BTreeMap<String, JsonData>>,
    entity: Entity,
    rpm: Option<i64>,
    tpm: Option<i64>,
) -> Result<(), KeyLimitError> {
    let rpm = model_ceilings(metadata, MODEL_RPM_FIELD, RateLimit::Rpm, entity, rpm)?;
    let tpm = model_ceilings(metadata, MODEL_TPM_FIELD, RateLimit::Tpm, entity, tpm)?;
    check_model_limits(keys, request, &rpm, &tpm)
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
    let allowed = &project.project.models;
    if let Some(models) = request.models().filter(|_| !allowed.is_empty()) {
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
    match field {
        Some(kind) => kind == LimitType::GuaranteedThroughput,
        None => matches!(
            request.metadata().and_then(|metadata| metadata.get(key)),
            Some(JsonData::String(text)) if text == GUARANTEED
        ),
    }
}

fn permitted(model: &JsonData, allowed: &[String]) -> bool {
    let JsonData::String(name) = model else {
        return false;
    };
    allowed.contains(name) || name == ALL_TEAM_MODELS || name == ALL_PROXY_MODELS
}
