use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorRateLimit, BatchLimitDescriptor,
};
use betterlite_core::api::batch_limit_descriptors::limiter::BatchLimiterSource;
use betterlite_core::api::request_destination::RequestData;
use betterlite_rate_limit::identity_descriptors::agent_descriptor::AgentDescriptorBuilder;
use betterlite_rate_limit::identity_descriptors::agent_registry::AgentPolicyRegistry;
use betterlite_rate_limit::limiter_records::{
    ModelGroupResolver, PROJECT_OTPM_DESCRIPTOR_KEY as OTPM_KEY, RateLimitDescriptor,
    RateLimitDescriptorRateLimit, TagRateLimit, TagRateLimitResolver,
};
use betterlite_rate_limit::model_descriptors::ModelDescriptorBuilder;
use betterlite_rate_limit::request_scopes::RequestScopeBuilder;
use betterlite_rate_limit::request_scopes::deployment_health::{
    DeploymentHealthError, DeploymentHealthPolicy, DeploymentHealthSource, DeploymentRef,
};
use betterlite_rate_limit::request_scopes::key_scopes::{
    KeyScopesError, ScopeInputs, create_rate_limit_descriptors as build_descriptors,
};
use betterlite_rate_limit::token_estimation::output_cap::{
    no_max_tokens_output_floor, output_candidate_count,
};

// Its state is the key, team, user and project limits, so the resolvers for the rest stay empty
static NO_DEPS: NoDeps = NoDeps;

pub(super) struct RateLimitSource {
    pub(super) window_size: i64,
}

struct NoDeps;

impl ModelGroupResolver for NoDeps {
    fn resolve(&self, _model: &str) -> Option<String> {
        None
    }
}

impl AgentPolicyRegistry for NoDeps {
    fn agent_by_id(&self, _agent_id: &str) -> Option<AgentResponse> {
        None
    }
}

#[async_trait]
impl DeploymentHealthSource for NoDeps {
    async fn deployments_for_model(
        &self,
        _model: &str,
    ) -> Result<Option<Vec<DeploymentRef>>, DeploymentHealthError> {
        Ok(None)
    }

    async fn failures_this_minute(
        &self,
        _deployment_id: &str,
    ) -> Result<i64, DeploymentHealthError> {
        Ok(0)
    }
}

#[async_trait]
impl TagRateLimitResolver for NoDeps {
    async fn resolve(&self, _tag_names: &[String]) -> BTreeMap<String, TagRateLimit> {
        BTreeMap::new()
    }
}

impl RateLimitSource {
    fn builder(&self) -> RequestScopeBuilder<'static> {
        RequestScopeBuilder {
            window_size: self.window_size,
            model: ModelDescriptorBuilder {
                window_size: self.window_size,
                resolver: Box::new(NoDeps),
            },
            agents: AgentDescriptorBuilder {
                window_size: self.window_size,
                registry: &NO_DEPS,
            },
            health: DeploymentHealthPolicy {
                source: &NO_DEPS,
                error_threshold_per_minute: 1,
            },
            tag_resolver: &NO_DEPS,
        }
    }
}

impl BatchLimiterSource for RateLimitSource {
    type Error = KeyScopesError;

    const PROJECT_OTPM_DESCRIPTOR_KEY: &'static str = OTPM_KEY;

    fn descriptors_for(
        &self,
        auth: &UserAPIKeyAuth,
        data: &RequestData,
    ) -> Result<Vec<BatchLimitDescriptor>, Self::Error> {
        let builder = self.builder();
        let inputs = ScopeInputs {
            auth,
            data,
            rpm_limit_type: None,
            tpm_limit_type: None,
            model_has_failures: false,
            call_type: None,
        };
        let descriptors = build_descriptors(&builder, &inputs)?;
        Ok(descriptors.into_iter().map(describe).collect())
    }

    fn project_io_for(
        &self,
        auth: &UserAPIKeyAuth,
        model: &str,
        out: &mut Vec<BatchLimitDescriptor>,
    ) {
        let builder = self.builder();
        let mut descriptors = Vec::new();
        builder
            .model
            .project_io_descriptors(auth, Some(model), &mut descriptors);
        out.extend(descriptors.into_iter().map(describe));
    }

    fn candidate_count(&self, body: &RequestData) -> i64 {
        output_candidate_count(body, None)
    }

    fn output_floor(&self, min_configured_otpm_limit: Option<i64>) -> i64 {
        no_max_tokens_output_floor(min_configured_otpm_limit)
    }
}

fn describe(descriptor: RateLimitDescriptor) -> BatchLimitDescriptor {
    BatchLimitDescriptor {
        key: descriptor.key,
        value: descriptor.value,
        rate_limit: descriptor.rate_limit.as_ref().map(limit_record),
    }
}

const fn limit_record(limit: &RateLimitDescriptorRateLimit) -> BatchDescriptorRateLimit {
    BatchDescriptorRateLimit {
        requests_per_unit: limit.requests_per_unit,
        tokens_per_unit: limit.tokens_per_unit,
        max_parallel_requests: limit.max_parallel_requests,
        window_size: limit.window_size,
    }
}
