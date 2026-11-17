#![cfg(test)]

use std::sync::Arc;

use crate::prompt_affinity::service::PromptAffinity;
use crate::prompt_affinity::tests::support::{
    FakePins, MARKED_MESSAGES, PLAIN_MESSAGES, cache_key, deployment, ids, injected_json,
    json_messages, messages, pin, request_kwargs, service,
};
use crate::router_core::candidates::CandidateSet;
use crate::router_core::services::RouteServices;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request,
};
use crate::static_eligibility::request::RequestFacts;

use super::apply;

// the check the server registers when the config names prompt_caching
struct Pinned {
    affinity: PromptAffinity,
}

impl RouteServices for Pinned {
    fn prompt_affinity(&self) -> Option<&PromptAffinity> {
        Some(&self.affinity)
    }
}

fn pool() -> CandidateSet {
    CandidateSet {
        deployments: vec![deployment("a", "claude-min"), deployment("b", "claude-min")],
    }
}

#[tokio::test]
async fn a_pinned_prefix_narrows_the_pool() {
    let pins = Arc::new(FakePins::default());
    pins.seed(
        &cache_key(&json_messages(MARKED_MESSAGES), None).unwrap(),
        pin("a"),
    );
    let services = Pinned {
        affinity: service(pins, 2000, false),
    };
    let facts = RequestFacts::default();
    let typed = messages(MARKED_MESSAGES);
    let mut request = route_request("gpt-4o", &facts);
    request.messages = Some(typed.as_slice());
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let narrowed = apply(pool(), &route).await.unwrap();
    assert_eq!(ids(&narrowed.deployments), ["a"]);
}

// a router that does not name the check leaves the pool to the pre-call checks and the strategy
#[tokio::test]
async fn a_router_without_the_check_keeps_the_pool() {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let kept = apply(pool(), &route).await.unwrap();
    assert_eq!(ids(&kept.deployments), ["a", "b"]);
}

// a fallback retry carries _target_order, which stands the check down and keeps the pool whole
#[tokio::test]
async fn a_target_order_keeps_the_pool() {
    let pins = Arc::new(FakePins::default());
    pins.seed(
        &cache_key(&json_messages(MARKED_MESSAGES), None).unwrap(),
        pin("a"),
    );
    let services = Pinned {
        affinity: service(pins, 2000, false),
    };
    let facts = RequestFacts {
        target_order: Some(2),
        ..RequestFacts::default()
    };
    let typed = messages(MARKED_MESSAGES);
    let mut request = route_request("gpt-4o", &facts);
    request.messages = Some(typed.as_slice());
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let kept = apply(pool(), &route).await.unwrap();
    assert_eq!(ids(&kept.deployments), ["a", "b"]);
}

// request arguments that enable prompt caching reach the check, so a pin under the injected breakpoint narrows
#[tokio::test]
async fn request_kwargs_enable_prompt_caching_narrows_the_pool() {
    let kwargs = request_kwargs(r#"{"enable_prompt_caching": true}"#);
    let injected = injected_json(PLAIN_MESSAGES, &kwargs, Some(true), false);
    let pins = Arc::new(FakePins::default());
    pins.seed(&cache_key(&injected, None).unwrap(), pin("a"));
    let services = Pinned {
        affinity: service(pins, 2000, false),
    };
    let facts = RequestFacts::default();
    let typed = messages(PLAIN_MESSAGES);
    let mut request = route_request("gpt-4o", &facts);
    request.messages = Some(typed.as_slice());
    request.request_kwargs = Some(&kwargs);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let narrowed = apply(pool(), &route).await.unwrap();
    assert_eq!(ids(&narrowed.deployments), ["a"]);
}
