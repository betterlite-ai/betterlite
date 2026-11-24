use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use serde::de::DeserializeOwned;

use crate::key_limit_policy::error::{Entity, KeyLimitError, Rejection};
use crate::key_limit_policy::request::KeyLimitRequest;
use crate::team_parent_limits::error::RateLimit;

pub const MODEL_RPM_FIELD: &str = "model_rpm_limit";
pub const MODEL_TPM_FIELD: &str = "model_tpm_limit";

// one metric's entity ceilings: the aggregate limit and the metadata per-model map
pub struct ModelCeilings {
    pub limit: RateLimit,
    pub entity: Entity,
    pub aggregate: Option<i64>,
    pub per_model: BTreeMap<String, Option<i64>>,
}

pub fn model_ceilings(
    metadata: Option<&BTreeMap<String, JsonData>>,
    field: &'static str,
    limit: RateLimit,
    entity: Entity,
    aggregate: Option<i64>,
) -> Result<ModelCeilings, KeyLimitError> {
    let stored = metadata.and_then(|entries| entries.get(field));
    let per_model = decode::<BTreeMap<String, Option<i64>>>(stored, field)?.unwrap_or_default();
    Ok(ModelCeilings {
        limit,
        entity,
        aggregate,
        per_model,
    })
}

pub fn check_model_limits(
    keys: &[LiteLLMVerificationToken],
    request: &KeyLimitRequest,
    rpm: &ModelCeilings,
    tpm: &ModelCeilings,
) -> Result<(), KeyLimitError> {
    let rpm_requested =
        requested_limits(request.model_rpm_limit.as_ref(), request, MODEL_RPM_FIELD)?;
    let tpm_requested =
        requested_limits(request.model_tpm_limit.as_ref(), request, MODEL_TPM_FIELD)?;
    if rpm_requested.is_none() && tpm_requested.is_none() {
        return Ok(());
    }
    let rpm_allocated = allocated_by_model(keys, MODEL_RPM_FIELD)?;
    let tpm_allocated = allocated_by_model(keys, MODEL_TPM_FIELD)?;
    if let Some(limits) = rpm_requested {
        check_models(&limits, &rpm_allocated, rpm)?;
    }
    if let Some(limits) = tpm_requested {
        check_models(&limits, &tpm_allocated, tpm)?;
    }
    Ok(())
}

pub fn check_rate_limits(
    keys: &[LiteLLMVerificationToken],
    request: &KeyLimitRequest,
    rpm_ceiling: Option<i64>,
    tpm_ceiling: Option<i64>,
    entity: Entity,
) -> Result<(), Rejection> {
    let limits = [
        (RateLimit::Tpm, request.tpm_limit, tpm_ceiling),
        (RateLimit::Rpm, request.rpm_limit, rpm_ceiling),
    ];
    for (limit, requested, ceiling) in limits {
        let (Some(asked), Some(cap)) = (requested, ceiling) else {
            continue;
        };
        let allocated = keys
            .iter()
            .filter_map(|key| match limit {
                RateLimit::Rpm => key.rpm_limit,
                RateLimit::Tpm => key.tpm_limit,
            })
            .map(i128::from)
            .fold(0_i128, i128::saturating_add);
        if i128::from(asked).saturating_add(allocated) > i128::from(cap) {
            return Err(Rejection::Allocation {
                limit,
                entity,
                allocated,
                requested: asked,
                ceiling: cap,
            });
        }
    }
    Ok(())
}

// the aggregate ceiling binds first; a per-model zero or null ceiling is skipped as LiteLLM's truthiness test does
fn check_models(
    requested: &BTreeMap<String, i64>,
    allocated: &BTreeMap<String, i128>,
    ceilings: &ModelCeilings,
) -> Result<(), Rejection> {
    for (model, asked) in requested {
        let already = allocated.get(model).copied().unwrap_or(0_i128);
        let per_model = ceilings
            .per_model
            .get(model)
            .copied()
            .flatten()
            .filter(|cap| *cap != 0);
        let exceeded = [ceilings.aggregate, per_model]
            .into_iter()
            .flatten()
            .find(|cap| i128::from(*asked).saturating_add(already) > i128::from(*cap));
        if let Some(cap) = exceeded {
            return Err(Rejection::Allocation {
                limit: ceilings.limit,
                entity: ceilings.entity,
                allocated: already,
                requested: *asked,
                ceiling: cap,
            });
        }
    }
    Ok(())
}

// team_parent_limits/allocation.rs runs the same four steps privately for team rows, so sharing them needs a scope change.
fn requested_limits(
    field_value: Option<&BTreeMap<String, JsonData>>,
    request: &KeyLimitRequest,
    field: &'static str,
) -> Result<Option<BTreeMap<String, i64>>, KeyLimitError> {
    // a request map with entries wins; an absent or empty one falls back to the request metadata
    if let Some(limits) = field_value.filter(|limits| !limits.is_empty()) {
        let stored = JsonData::Object(limits.clone());
        return decode(Some(&stored), field);
    }
    let stored = request
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(field));
    decode(stored, field)
}

fn allocated_by_model(
    keys: &[LiteLLMVerificationToken],
    field: &'static str,
) -> Result<BTreeMap<String, i128>, KeyLimitError> {
    let mut allocated = BTreeMap::new();
    for key in keys {
        let Some(limits) = decode::<BTreeMap<String, i64>>(key.metadata.0.get(field), field)?
        else {
            continue;
        };
        for (model, limit) in limits {
            let total = allocated.entry(model).or_insert(0_i128);
            *total = total.saturating_add(i128::from(limit));
        }
    }
    Ok(allocated)
}

fn decode<T>(stored: Option<&JsonData>, field: &'static str) -> Result<Option<T>, KeyLimitError>
where
    T: DeserializeOwned,
{
    let Some(found) = stored else {
        return Ok(None);
    };
    serde_json::to_value(found)
        .and_then(serde_json::from_value::<Option<T>>)
        .map_err(|source| KeyLimitError::ModelLimits { field, source })
}
