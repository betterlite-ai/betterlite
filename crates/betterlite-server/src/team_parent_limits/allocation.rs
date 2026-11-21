use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use serde::de::DeserializeOwned;

use super::error::{RateLimit, Rejection, TeamLimitError};
use super::request::TeamLimitRequest;

type ModelLimits = BTreeMap<String, i64>;

const MODEL_RPM_FIELD: &str = "model_rpm_limit";
const MODEL_TPM_FIELD: &str = "model_tpm_limit";

// one metric's organization ceilings: the budget-table total and the metadata per-model map
struct ModelCeilings {
    limit: RateLimit,
    aggregate: Option<i64>,
    per_model: BTreeMap<String, Option<i64>>,
}

pub fn check_org_team_model_specific_limits(
    teams: &[LiteLLMTeamTable],
    organization: &OrganizationTable,
    request: &TeamLimitRequest<'_>,
) -> Result<(), TeamLimitError> {
    let rpm_requested = requested_limits(request.model_rpm_limit, request, MODEL_RPM_FIELD)?;
    let tpm_requested = requested_limits(request.model_tpm_limit, request, MODEL_TPM_FIELD)?;
    if rpm_requested.is_none() && tpm_requested.is_none() {
        return Ok(());
    }
    let rpm_allocated = allocated_by_model(teams, MODEL_RPM_FIELD)?;
    let tpm_allocated = allocated_by_model(teams, MODEL_TPM_FIELD)?;
    let budget = organization.litellm_budget_table.as_ref();
    if let Some(limits) = rpm_requested {
        let ceilings = ModelCeilings {
            limit: RateLimit::Rpm,
            aggregate: budget.and_then(|table| table.rpm_limit),
            per_model: organization_ceilings(organization, MODEL_RPM_FIELD)?,
        };
        check_models(&limits, &rpm_allocated, &ceilings)?;
    }
    if let Some(limits) = tpm_requested {
        let ceilings = ModelCeilings {
            limit: RateLimit::Tpm,
            aggregate: budget.and_then(|table| table.tpm_limit),
            per_model: organization_ceilings(organization, MODEL_TPM_FIELD)?,
        };
        check_models(&limits, &tpm_allocated, &ceilings)?;
    }
    Ok(())
}

pub fn check_org_team_rpm_tpm_limits(
    teams: &[LiteLLMTeamTable],
    organization: &OrganizationTable,
    request: &TeamLimitRequest<'_>,
) -> Result<(), Rejection> {
    let budget = organization.litellm_budget_table.as_ref();
    let allocated_tpm = teams
        .iter()
        .filter_map(|team| team.fields.base.tpm_limit)
        .map(i128::from)
        .fold(0_i128, i128::saturating_add);
    let allocated_rpm = teams
        .iter()
        .filter_map(|team| team.fields.base.rpm_limit)
        .map(i128::from)
        .fold(0_i128, i128::saturating_add);
    let tpm_ceiling = budget.and_then(|table| table.tpm_limit);
    check_total(
        RateLimit::Tpm,
        request.tpm_limit,
        allocated_tpm,
        tpm_ceiling,
    )?;
    let rpm_ceiling = budget.and_then(|table| table.rpm_limit);
    check_total(
        RateLimit::Rpm,
        request.rpm_limit,
        allocated_rpm,
        rpm_ceiling,
    )
}

// i128 holds any sum of i64 limits, so the comparison never wraps where Python's int would not
fn check_total(
    limit: RateLimit,
    requested: Option<i64>,
    allocated: i128,
    ceiling: Option<i64>,
) -> Result<(), Rejection> {
    if let (Some(asked), Some(cap)) = (requested, ceiling)
        && i128::from(asked).saturating_add(allocated) > i128::from(cap)
    {
        return Err(Rejection::Allocation {
            limit,
            allocated,
            requested: asked,
            ceiling: cap,
        });
    }
    Ok(())
}

// the budget-table total binds first; a zero or null metadata ceiling is skipped as LiteLLM's truthiness test does
fn check_models(
    requested: &ModelLimits,
    allocated: &BTreeMap<String, i128>,
    ceilings: &ModelCeilings,
) -> Result<(), Rejection> {
    for (model, asked) in requested {
        let already = allocated.get(model).copied().unwrap_or(0_i128);
        check_total(ceilings.limit, Some(*asked), already, ceilings.aggregate)?;
        let per_model = ceilings.per_model.get(model).copied().flatten();
        let cap = per_model.filter(|value| *value != 0_i64);
        check_total(ceilings.limit, Some(*asked), already, cap)?;
    }
    Ok(())
}

// a request map with entries wins; an absent or empty one falls back to the request metadata
fn requested_limits(
    field_value: Option<&ModelLimits>,
    request: &TeamLimitRequest<'_>,
    field: &'static str,
) -> Result<Option<ModelLimits>, TeamLimitError> {
    if let Some(limits) = field_value.filter(|limits| !limits.is_empty()) {
        return Ok(Some(limits.clone()));
    }
    let stored = request.metadata.and_then(|metadata| metadata.get(field));
    decode(stored, field)
}

fn allocated_by_model(
    teams: &[LiteLLMTeamTable],
    field: &'static str,
) -> Result<BTreeMap<String, i128>, TeamLimitError> {
    let mut allocated = BTreeMap::new();
    for team in teams {
        let metadata = team.fields.base.metadata.as_ref();
        let stored = metadata.and_then(|entries| entries.get(field));
        let Some(limits) = decode::<ModelLimits>(stored, field)? else {
            continue;
        };
        for (model, limit) in limits {
            let total = allocated.entry(model).or_insert(0_i128);
            *total = total.saturating_add(i128::from(limit));
        }
    }
    Ok(allocated)
}

fn organization_ceilings(
    organization: &OrganizationTable,
    field: &'static str,
) -> Result<BTreeMap<String, Option<i64>>, TeamLimitError> {
    let metadata = organization.metadata.as_ref();
    let stored = metadata.and_then(|payload| payload.0.get(field));
    let ceilings: Option<BTreeMap<String, Option<i64>>> = decode(stored, field)?;
    Ok(ceilings.unwrap_or_default())
}

fn decode<T>(stored: Option<&JsonData>, field: &'static str) -> Result<Option<T>, TeamLimitError>
where
    T: DeserializeOwned,
{
    let Some(found) = stored else {
        return Ok(None);
    };
    serde_json::to_value(found)
        .and_then(serde_json::from_value::<Option<T>>)
        .map_err(|source| TeamLimitError::ModelLimits { field, source })
}
