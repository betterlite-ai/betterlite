#![cfg(test)]

use std::convert::Infallible;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::error::RoutingError;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request, strategy_context,
};
use crate::static_eligibility::request::RequestFacts;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{Resolution, ResolutionSource};

use super::select;

// A sampled rng that answers the same u32 draw each time: 1 is the first index every range accepts and u32::MAX is
// its last.
struct Fixed(u32);

impl rand::TryRng for Fixed {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(self.0)
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        Ok(u64::from(self.0))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        for slot in dst.iter_mut() {
            *slot = 0;
        }
        Ok(())
    }
}

fn deployment(id: &str, params: &[(&str, Json)]) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), Json::Text("gpt-4o".to_owned()));
    for (name, value) in params {
        litellm_params.insert((*name).to_owned(), value.clone());
    }
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    let generate = || String::from("generated");
    DeploymentSpec::new(
        "gpt-4o".to_owned(),
        litellm_params,
        model_info,
        JsonObject::new(),
        generate,
    )
    .expect("the test deployment is valid")
}

async fn pick(
    pool: &[DeploymentSpec],
    facts: &RequestFacts,
    random: &mut Fixed,
) -> Result<usize, RoutingError> {
    let request = route_request("gpt-4o", facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::UsageBasedRoutingV2, None).unwrap();
    let resolution = Resolution {
        strategy: RoutingStrategy::UsageBasedRoutingV2,
        selector: None,
        source: ResolutionSource::Default,
        bound: false,
        ignored_override: None,
    };
    let context = strategy_context(pool, &route, &mut dispatch, &resolution);
    select(&context, random).await
}

#[tokio::test]
async fn the_cold_pick_leaves_the_pool_to_the_rng() {
    let pool = [
        deployment("a", &[]),
        deployment("b", &[]),
        deployment("c", &[]),
    ];
    let facts = RequestFacts::default();
    let first = pick(&pool, &facts, &mut Fixed(0)).await.unwrap();
    assert_eq!(first, 0);
    let last = pick(&pool, &facts, &mut Fixed(u32::MAX)).await.unwrap();
    assert_eq!(last, 2);
}

#[tokio::test]
async fn the_request_tokens_screen_a_cold_deployment() {
    let pool = [deployment(
        "a",
        &[("tpm", Json::Number(serde_json::Number::from(5)))],
    )];
    let roomy = RequestFacts {
        input_token_count: Some(5),
        ..RequestFacts::default()
    };
    assert_eq!(pick(&pool, &roomy, &mut Fixed(0)).await.unwrap(), 0);
    let over = RequestFacts {
        input_token_count: Some(6),
        ..RequestFacts::default()
    };
    assert_eq!(pick(&pool, &over, &mut Fixed(0)).await.unwrap(), 1);
}

#[tokio::test]
async fn an_empty_pool_answers_the_end_of_the_pool() {
    let facts = RequestFacts::default();
    let selected = pick(&[], &facts, &mut Fixed(0)).await.unwrap();
    assert_eq!(selected, 0);
}

#[tokio::test]
async fn a_limit_that_is_not_a_number_names_its_field() {
    let pool = [deployment("a", &[("tpm", Json::Text("many".to_owned()))])];
    let facts = RequestFacts::default();
    let error = pick(&pool, &facts, &mut Fixed(0)).await.unwrap_err();
    assert!(matches!(error, RoutingError::Metric { field: "tpm" }));
}
