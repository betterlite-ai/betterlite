use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::auth_limit_inputs::key_limits::ModelRateLimitKey;
use betterlite_auth::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, model_limit_from_metadata,
};
use betterlite_auth::auth_model_patterns::ALL_PROXY_MODELS;
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::BatchLimitDescriptors;
use super::fields::BatchPrincipalFields;
use super::input::BatchModelUsage;
use super::limiter::BatchLimiterSource;
use crate::api::request_destination::RequestData;

pub const TPD_SUFFIX: &str = "_tpd";
pub const TPD_WINDOW_SECONDS: i64 = 86_400;

#[derive(Debug, Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatchDescriptorRateLimit {
    pub requests_per_unit: Option<i64>,
    pub tokens_per_unit: Option<i64>,
    pub max_parallel_requests: Option<i64>,
    pub window_size: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BatchLimitDescriptor {
    pub key: String,
    pub value: String,
    pub rate_limit: Option<BatchDescriptorRateLimit>,
}

// The charge a project ITPM/OTPM descriptor carries; requests stay at zero, so only tokens move a counter
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize)]
pub struct BatchDescriptorIncrements {
    pub requests: i64,
    pub tokens: i64,
}

// LiteLLM's base descriptors with each tpd scope swapped for its daily-token descriptor
pub(super) fn create_batch_descriptors<Limiter, Credentials>(
    service: &BatchLimitDescriptors<Limiter, Credentials>,
    fields: &BatchPrincipalFields,
    auth: &UserAPIKeyAuth,
    data: &RequestData,
) -> Result<Vec<BatchLimitDescriptor>, Limiter::Error>
where
    Limiter: BatchLimiterSource,
{
    let mut descriptors = service.limiter.descriptors_for(auth, data)?;
    let account = account_limits(auth);
    let view = auth.request_quota_view();
    let scopes: Vec<(&'static str, String, i64)> = [
        ("api_key", account.hashed_api_key, fields.tpd_limit),
        ("team", account.team_id, fields.team_tpd_limit),
        ("end_user", view.end_user_id, fields.end_user_tpd_limit),
    ]
    .into_iter()
    .filter_map(|(scope, value, limit)| {
        let bound = value.filter(|text| !text.is_empty())?;
        Some((scope, bound.to_owned(), limit?))
    })
    .collect();
    descriptors.retain(|descriptor| {
        scopes
            .iter()
            .all(|(scope, _, _)| descriptor.key.as_str() != *scope)
    });
    descriptors.extend(
        scopes
            .into_iter()
            .map(|(scope, value, limit)| BatchLimitDescriptor {
                key: format!("{scope}{TPD_SUFFIX}"),
                value,
                rate_limit: Some(BatchDescriptorRateLimit {
                    requests_per_unit: None,
                    tokens_per_unit: Some(limit),
                    max_parallel_requests: None,
                    window_size: Some(TPD_WINDOW_SECONDS),
                }),
            }),
    );
    Ok(descriptors)
}

// One project ITPM/OTPM pair per row model, each charged that model's own counted tokens
pub(super) fn project_io_descriptors<Limiter, Credentials>(
    service: &BatchLimitDescriptors<Limiter, Credentials>,
    auth: &UserAPIKeyAuth,
    per_model_usage: &IndexMap<String, BatchModelUsage>,
) -> (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>)
where
    Limiter: BatchLimiterSource,
{
    let mut descriptors = Vec::new();
    let mut increments = Vec::new();
    for (model, usage) in per_model_usage {
        let mut model_descriptors = Vec::new();
        service
            .limiter
            .project_io_for(auth, model, &mut model_descriptors);
        for descriptor in model_descriptors {
            let tokens = if descriptor.key == Limiter::PROJECT_OTPM_DESCRIPTOR_KEY {
                usage.output_tokens
            } else {
                usage.total_tokens
            };
            increments.push(BatchDescriptorIncrements {
                requests: 0,
                tokens,
            });
            descriptors.push(descriptor);
        }
    }
    (descriptors, increments)
}

// LiteLLM reads the project's per-model quota maps, not one entry: any model may be charged by a row
#[must_use]
pub fn project_has_io_limits(auth: &UserAPIKeyAuth) -> bool {
    if account_limits(auth).project_id.is_none() {
        return false;
    }
    let itpm = model_limit_from_metadata(auth, MetadataAccessor::Project, ModelRateLimitKey::Itpm);
    let otpm = model_limit_from_metadata(auth, MetadataAccessor::Project, ModelRateLimitKey::Otpm);
    // an empty map is falsy in LiteLLM, so a quota key with no entries configures nothing
    itpm.is_some_and(|limits| !limits.is_empty()) || otpm.is_some_and(|limits| !limits.is_empty())
}

// LiteLLM's wildcard and all-proxy-models entries lift the restriction; any other non-empty list keeps it
#[must_use]
pub fn key_requires_access_check(fields: &BatchPrincipalFields) -> bool {
    let models = fields.models.as_deref().unwrap_or(&[]);
    if models
        .iter()
        .any(|model| model == "*" || model == ALL_PROXY_MODELS)
    {
        return false;
    }
    let groups_restrict = fields
        .access_group_ids
        .as_deref()
        .is_some_and(|groups| !groups.is_empty());
    groups_restrict || !models.is_empty()
}

#[must_use]
pub fn has_applicable_limits(descriptors: &[BatchLimitDescriptor]) -> bool {
    descriptors.iter().any(|descriptor| {
        descriptor.rate_limit.as_ref().is_some_and(|rate_limit| {
            rate_limit.requests_per_unit.is_some()
                || rate_limit.tokens_per_unit.is_some()
                || rate_limit.max_parallel_requests.is_some()
        })
    })
}
