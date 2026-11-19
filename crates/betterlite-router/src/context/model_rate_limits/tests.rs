#![cfg(test)]

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::reservation::RequestBody;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::error::{IoChannel, IoPreCallError, RateLimitExceeded};
use crate::io_token_reservations::limits::IoDeployment;
use crate::io_token_reservations::reservation::IoReservation;
use crate::model_rate_limits::service::IoTokenReservationHook;
use crate::model_rate_limits::tests::support::{
    FakeCache, WithRateLimits, rpm_key, service, spec, tpm_key, with_extra, with_param,
};
use crate::router_core::candidates::CandidateSet;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request,
};
use crate::static_eligibility::request::RequestFacts;

use super::apply;

fn registered(cache: FakeCache) -> WithRateLimits {
    WithRateLimits {
        check: service(Arc::new(cache), None),
    }
}

fn pool(deployment: DeploymentSpec) -> CandidateSet {
    CandidateSet {
        deployments: vec![deployment],
    }
}

#[tokio::test]
async fn an_unregistered_check_keeps_the_candidate() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let deployment = with_param(spec("gpt-4o", "a", "openai/a"), "tpm", 1);
    let kept = apply(pool(deployment.clone()), &route).await.unwrap();
    assert_eq!(kept.deployments, [deployment]);
}

// a candidate whose current tpm reached its limit is left out, as a rate limit stops pre_call_check
#[tokio::test]
async fn a_candidate_at_its_tpm_limit_is_dropped() {
    let cache = FakeCache::new().seed_local(&tpm_key("openai/a"), 1);
    let services = registered(cache);
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let deployment = with_param(spec("gpt-4o", "a", "openai/a"), "tpm", 1);
    let kept = apply(pool(deployment), &route).await.unwrap();
    assert!(kept.deployments.is_empty());
}

// an under-limit candidate stays, and the check counts its attempt before the caller picks it
#[tokio::test]
async fn an_under_limit_candidate_stays_and_counts() {
    let cache = Arc::new(FakeCache::new());
    let services = WithRateLimits {
        check: service(Arc::clone(&cache), None),
    };
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let deployment = with_param(spec("gpt-4o", "a", "openai/a"), "rpm", 1);
    let kept = apply(pool(deployment.clone()), &route).await.unwrap();
    assert_eq!(kept.deployments, [deployment]);
    assert_eq!(cache.local_value(&rpm_key("openai/a")), Some(1));
}

// a limit the check cannot read is swallowed, as pre_call_check returns the deployment on any other exception
#[tokio::test]
async fn an_unreadable_limit_keeps_the_candidate() {
    let services = registered(FakeCache::new());
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let deployment = with_extra(
        spec("gpt-4o", "a", "openai/a"),
        "tpm",
        Json::Text(String::from("1")),
    );
    let kept = apply(pool(deployment.clone()), &route).await.unwrap();
    assert_eq!(kept.deployments, [deployment]);
}

// The refusal B019's io-token reservation hands back: its own rate limit, or a limit it cannot read
enum Refusal {
    RateLimit,
    OutputCap,
}

struct RefusingIo(Refusal);

#[async_trait]
impl IoTokenReservationHook for RefusingIo {
    async fn reserve(
        &self,
        _deployment: &IoDeployment<'_>,
        _request: &mut RequestBody,
    ) -> Result<Option<IoReservation>, IoPreCallError> {
        match self.0 {
            Refusal::RateLimit => Err(IoPreCallError::RateLimit(RateLimitExceeded {
                channel: IoChannel::Itpm,
                limit: 10,
                current: 10,
            })),
            Refusal::OutputCap => Err(IoPreCallError::OutputCap { field: "itpm" }),
        }
    }

    fn refund(&self, _request: &mut RequestBody) {}

    async fn async_refund(&self, _request: &mut RequestBody) {}
}

// a reservation that hits its own io-token limit rejects the candidate, as the check re-raises a rate limit
#[tokio::test]
async fn an_io_token_rate_limit_drops_the_candidate() {
    let services = WithRateLimits {
        check: service(
            Arc::new(FakeCache::new()),
            Some(Arc::new(RefusingIo(Refusal::RateLimit))),
        ),
    };
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let deployment = with_param(spec("gpt-4o", "a", "openai/a"), "itpm", 10);
    let kept = apply(pool(deployment), &route).await.unwrap();
    assert!(kept.deployments.is_empty());
}

// an io-token refusal that is not a rate limit is swallowed, as pre_call_check returns the deployment
#[tokio::test]
async fn an_unreadable_io_token_limit_keeps_the_candidate() {
    let services = WithRateLimits {
        check: service(
            Arc::new(FakeCache::new()),
            Some(Arc::new(RefusingIo(Refusal::OutputCap))),
        ),
    };
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let deployment = with_param(spec("gpt-4o", "a", "openai/a"), "itpm", 10);
    let kept = apply(pool(deployment.clone()), &route).await.unwrap();
    assert_eq!(kept.deployments, [deployment]);
}
