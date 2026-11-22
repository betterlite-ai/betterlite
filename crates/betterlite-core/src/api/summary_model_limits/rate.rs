use std::error::Error;
use std::future::Future;

use betterlite_auth::resolved_key::auth::{ParentOtelSpan, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};

use super::error::{LimiterFailure, SummaryLimitsError};
use super::fields::LimitFields;

pub const PARALLEL_REQUEST_LIMITER: &str = "parallel_request_limiter";
pub const OVER_LIMIT: &str = "OVER_LIMIT";

// the limiter attributes compact_20260112 looks up by name; a limiter without one of them is not checked
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LimiterCapability {
    #[serde(rename = "should_rate_limit")]
    ShouldRateLimit,
    #[serde(rename = "_create_rate_limit_descriptors")]
    CreateDescriptors,
    #[serde(rename = "_add_team_model_rate_limit_descriptor_from_metadata")]
    AddTeamModelDescriptor,
    #[serde(rename = "_add_project_model_rate_limit_descriptor_from_metadata")]
    AddProjectModelDescriptor,
    #[serde(rename = "create_organization_rate_limit_descriptor")]
    CreateOrganizationDescriptors,
}

pub const LIMITER_CAPABILITIES: [LimiterCapability; 5] = [
    LimiterCapability::ShouldRateLimit,
    LimiterCapability::CreateDescriptors,
    LimiterCapability::AddTeamModelDescriptor,
    LimiterCapability::AddProjectModelDescriptor,
    LimiterCapability::CreateOrganizationDescriptors,
];

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DescriptorLimits {
    pub requests_per_unit: Option<i64>,
    pub tokens_per_unit: Option<i64>,
    pub max_parallel_requests: Option<i64>,
    pub window_size: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryDescriptor {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub rate_limit: Option<DescriptorLimits>,
}

// the keyword arguments _create_rate_limit_descriptors gets: data is {"model": model}
pub struct BaseDescriptorRequest<'call> {
    pub principal: &'call UserAPIKeyAuth,
    pub model: &'call str,
    pub rpm_limit_type: Option<&'call JsonData>,
    pub tpm_limit_type: Option<&'call JsonData>,
    pub model_has_failures: bool,
}

// team and project descriptors join the base list in place; organization descriptors are appended after them
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorScope {
    Team,
    Project,
    Organization,
}

pub struct ReadOnlyCheck<'call> {
    pub descriptors: &'call [SummaryDescriptor],
    pub parent_otel_span: Option<&'call ParentOtelSpan>,
    pub read_only: bool,
}

// proxy_logging_obj.get_proxy_hook
pub trait ProxyHooks: Sync {
    type Limiter: SummaryRateLimiter;
    type Error: Error + 'static;

    fn get_proxy_hook(&self, hook: &str) -> Result<Option<&Self::Limiter>, Self::Error>;
}

// the read-only part of parallel_request_limiter_v3: none of these reserves or increments a counter
pub trait SummaryRateLimiter: Sync {
    type Failure: Error + Send + 'static;

    fn supports(&self, capability: LimiterCapability) -> bool;

    fn create_rate_limit_descriptors(
        &self,
        request: &BaseDescriptorRequest<'_>,
    ) -> Result<Vec<SummaryDescriptor>, LimiterFailure<Self::Failure>>;

    fn add_scope_descriptors(
        &self,
        scope: DescriptorScope,
        principal: &UserAPIKeyAuth,
        requested_model: &str,
        descriptors: &mut Vec<SummaryDescriptor>,
    ) -> Result<(), LimiterFailure<Self::Failure>>;

    // the response's overall_code, absent when the response carries none
    fn should_rate_limit(
        &self,
        check: &ReadOnlyCheck<'_>,
    ) -> impl Future<Output = Result<Option<String>, LimiterFailure<Self::Failure>>> + Send;
}

// only OVER_LIMIT and an unverifiable counter deny; any other limiter failure lets the summary run
pub async fn summary_within_rate_limit<Hooks>(
    principal: Option<&UserAPIKeyAuth>,
    hooks: Option<&Hooks>,
    summary_model: &str,
) -> Result<bool, SummaryLimitsError<Hooks::Error>>
where
    Hooks: ProxyHooks,
{
    let (Some(auth), Some(proxy)) = (principal, hooks) else {
        return Ok(true);
    };
    let found = proxy
        .get_proxy_hook(PARALLEL_REQUEST_LIMITER)
        .map_err(SummaryLimitsError::Hook)?;
    let Some(limiter) = found.filter(|limiter| {
        LIMITER_CAPABILITIES
            .iter()
            .all(|capability| limiter.supports(*capability))
    }) else {
        return Ok(true);
    };
    let fields: LimitFields = serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .map_err(SummaryLimitsError::Principal)?;
    match read_only_check(auth, &fields, limiter, summary_model).await {
        Ok(allowed) => Ok(allowed),
        Err(LimiterFailure::Unverifiable { detail }) => {
            tracing::warn!(
                "compact_20260112: rate-limit counters for summary_model={summary_model} could not be verified; denying: {detail}"
            );
            Ok(false)
        }
        Err(LimiterFailure::Other(failure)) => {
            tracing::warn!(
                "compact_20260112: unexpected error during rate-limit check for summary_model={summary_model}; allowing: {failure}"
            );
            Ok(true)
        }
    }
}

async fn read_only_check<Limiter>(
    principal: &UserAPIKeyAuth,
    fields: &LimitFields,
    limiter: &Limiter,
    summary_model: &str,
) -> Result<bool, LimiterFailure<Limiter::Failure>>
where
    Limiter: SummaryRateLimiter,
{
    let metadata = fields.metadata.as_ref();
    let request = BaseDescriptorRequest {
        principal,
        model: summary_model,
        rpm_limit_type: metadata.and_then(|values| values.get("rpm_limit_type")),
        tpm_limit_type: metadata.and_then(|values| values.get("tpm_limit_type")),
        model_has_failures: false,
    };
    let mut descriptors = limiter.create_rate_limit_descriptors(&request)?;
    for scope in [
        DescriptorScope::Team,
        DescriptorScope::Project,
        DescriptorScope::Organization,
    ] {
        limiter.add_scope_descriptors(scope, principal, summary_model, &mut descriptors)?;
    }
    // the summary runs inside the caller's admitted request, so its own parallel slot would count against it
    for limits in descriptors
        .iter_mut()
        .filter_map(|descriptor| descriptor.rate_limit.as_mut())
    {
        limits.max_parallel_requests = None;
    }
    if descriptors.is_empty() {
        return Ok(true);
    }
    let check = ReadOnlyCheck {
        descriptors: &descriptors,
        parent_otel_span: fields.parent_otel_span.as_ref(),
        read_only: true,
    };
    let overall_code = limiter.should_rate_limit(&check).await?;
    Ok(overall_code.as_deref() != Some(OVER_LIMIT))
}
