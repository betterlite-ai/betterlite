use std::future::Future as _;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use betterlite_models::decisions::Json;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde_json::Number;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::error::RoutingError;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request, strategy_context,
};
use crate::static_eligibility::request::RequestFacts;
use crate::strategy::Strategy;
use crate::strategy::select as strategy_select;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{Resolution, ResolutionSource};
use crate::usage_routing::select::{WindowCounts, select, select_counts};

use super::support::{LocalWindows, counts, deployment, instant, number, plain, text};

const GROUP: &str = "gpt-4o";
const NOW: i64 = 1_700_000_000_000;

struct Boundary {
    tpm: Option<i64>,
    rpm: Option<i64>,
    seed: &'static [(&'static str, i64)],
    requests: &'static [(&'static str, i64)],
    input_tokens: i64,
}

fn select_one(boundary: &Boundary) -> Result<Option<usize>, RoutingError> {
    let mut limits = Vec::new();
    if let Some(tpm) = boundary.tpm {
        limits.push(("tpm", number(tpm)));
    }
    if let Some(rpm) = boundary.rpm {
        limits.push(("rpm", number(rpm)));
    }
    let deployments = [deployment("dep-a", &[], &[], limits.as_slice())];
    let stored_tokens = counts(boundary.seed);
    let stored_requests = counts(boundary.requests);
    let windows = WindowCounts {
        tpm: Some(&stored_tokens),
        rpm: Some(&stored_requests),
    };
    select_counts(&windows, &deployments, boundary.input_tokens)
}

// The estimate is checked with a strict greater-than, the next request is counted first, and an absent id is never ruled out.
#[rstest]
#[case::tokens_under_the_limit(Boundary { tpm: Some(10), rpm: None, seed: &[("dep-a", 0)], requests: &[], input_tokens: 9 }, Some(0))]
#[case::tokens_exactly_at_the_limit(Boundary { tpm: Some(10), rpm: None, seed: &[("dep-a", 0)], requests: &[], input_tokens: 10 }, Some(0))]
#[case::tokens_one_token_over_the_limit(Boundary { tpm: Some(10), rpm: None, seed: &[("dep-a", 0)], requests: &[], input_tokens: 11 }, None)]
#[case::tokens_already_over_the_limit(Boundary { tpm: Some(10), rpm: None, seed: &[("dep-a", 11)], requests: &[], input_tokens: 0 }, None)]
#[case::a_zero_token_limit_and_a_zero_estimate(Boundary { tpm: Some(0), rpm: None, seed: &[("dep-a", 0)], requests: &[], input_tokens: 0 }, Some(0))]
#[case::a_zero_token_limit_and_one_token(Boundary { tpm: Some(0), rpm: None, seed: &[("dep-a", 0)], requests: &[], input_tokens: 1 }, None)]
#[case::requests_absent_from_the_window(Boundary { tpm: None, rpm: Some(3), seed: &[("dep-a", 0)], requests: &[], input_tokens: 0 }, Some(0))]
#[case::requests_one_below_the_limit(Boundary { tpm: None, rpm: Some(3), seed: &[("dep-a", 0)], requests: &[("dep-a", 1)], input_tokens: 0 }, Some(0))]
#[case::the_next_request_reaches_the_limit(Boundary { tpm: None, rpm: Some(3), seed: &[("dep-a", 0)], requests: &[("dep-a", 2)], input_tokens: 0 }, None)]
#[case::requests_at_the_limit(Boundary { tpm: None, rpm: Some(3), seed: &[("dep-a", 0)], requests: &[("dep-a", 3)], input_tokens: 0 }, None)]
#[case::a_zero_request_limit_before_any_request(Boundary { tpm: None, rpm: Some(0), seed: &[("dep-a", 0)], requests: &[], input_tokens: 0 }, Some(0))]
#[case::a_zero_request_limit_after_one_request(Boundary { tpm: None, rpm: Some(0), seed: &[("dep-a", 0)], requests: &[("dep-a", 1)], input_tokens: 0 }, None)]
fn usage_routing_boundaries(#[case] boundary: Boundary, #[case] expected: Option<usize>) {
    assert_eq!(select_one(&boundary).unwrap(), expected);
}

#[test]
fn usage_routing_boundaries_treat_a_missing_limit_as_unbounded() {
    let deployments = [plain("dep-a")];
    let stored = counts(&[("dep-a", i64::MAX)]);
    let windows = WindowCounts {
        tpm: Some(&stored),
        rpm: Some(&stored),
    };
    assert_eq!(
        select_counts(&windows, &deployments, i64::MAX).unwrap(),
        Some(0)
    );
}

// an explicit null limit reads like an absent one, so neither check rules the deployment out
#[test]
fn usage_routing_boundaries_treat_a_null_limit_as_unbounded() {
    let deployments = [deployment(
        "dep-a",
        &[],
        &[],
        &[("tpm", Json::Null), ("rpm", Json::Null)],
    )];
    let stored = counts(&[("dep-a", 5)]);
    let windows = WindowCounts {
        tpm: Some(&stored),
        rpm: Some(&stored),
    };
    assert_eq!(select_counts(&windows, &deployments, 0).unwrap(), Some(0));
}

// a number that is not a whole one is a limit this field cannot hold, and the error names the field
#[test]
fn usage_routing_boundaries_name_a_fractional_token_limit() {
    let fractional = Number::from_f64(1.5).expect("1.5 is a finite number");
    let deployments = [deployment(
        "dep-a",
        &[],
        &[],
        &[("tpm", Json::Number(fractional))],
    )];
    let error = select_counts(&WindowCounts::default(), &deployments, 0).unwrap_err();
    assert!(matches!(error, RoutingError::Metric { field: "tpm" }));
    assert_eq!(error.unbuilt_entry(), None);
}

// a null at one level falls through to the next, the way dict.get() hands back None and LiteLLM reads on
#[test]
fn usage_routing_boundaries_read_past_a_null_limit_level() {
    let deployments = [deployment(
        "dep-a",
        &[("tpm", Json::Null)],
        &[],
        &[("tpm", number(10))],
    )];
    assert_eq!(
        select_counts(&WindowCounts::default(), &deployments, 11).unwrap(),
        None
    );
}

// the top-level 10 rules the request out before litellm_params' 1000 can be reached
#[test]
fn usage_routing_boundaries_read_the_top_token_limit_before_litellm_params() {
    let deployments = [deployment(
        "dep-a",
        &[("tpm", number(10))],
        &[("tpm", number(1000))],
        &[],
    )];
    assert_eq!(
        select_counts(&WindowCounts::default(), &deployments, 11).unwrap(),
        None
    );
}

#[test]
fn usage_routing_boundaries_read_the_litellm_params_token_limit_before_model_info() {
    let deployments = [deployment(
        "dep-a",
        &[],
        &[("tpm", number(10))],
        &[("tpm", number(1000))],
    )];
    assert_eq!(
        select_counts(&WindowCounts::default(), &deployments, 11).unwrap(),
        None
    );
}

// the same order decides the request limit, where a stored 9 plus the next request reaches 10
#[test]
fn usage_routing_boundaries_read_the_top_request_limit_before_litellm_params() {
    let deployments = [deployment(
        "dep-a",
        &[("rpm", number(10))],
        &[("rpm", number(1000))],
        &[],
    )];
    let stored = counts(&[("dep-a", 9)]);
    let windows = WindowCounts {
        tpm: None,
        rpm: Some(&stored),
    };
    assert_eq!(select_counts(&windows, &deployments, 0).unwrap(), None);
}

#[test]
fn usage_routing_boundaries_read_the_litellm_params_request_limit_before_model_info() {
    let deployments = [deployment(
        "dep-a",
        &[],
        &[("rpm", number(10))],
        &[("rpm", number(1000))],
    )];
    let stored = counts(&[("dep-a", 9)]);
    let windows = WindowCounts {
        tpm: None,
        rpm: Some(&stored),
    };
    assert_eq!(select_counts(&windows, &deployments, 0).unwrap(), None);
}

// a requests window without a token window still applies its limit, as it does when only the tpm key expired
#[test]
fn usage_routing_boundaries_apply_the_request_limit_without_a_token_window() {
    let deployments = [deployment("dep-a", &[], &[], &[("rpm", number(3))])];
    let requests = counts(&[("dep-a", 2)]);
    let windows = WindowCounts {
        tpm: None,
        rpm: Some(&requests),
    };
    assert_eq!(select_counts(&windows, &deployments, 0).unwrap(), None);
}

// a count and an estimate that add past i64's maximum still exceed the limit instead of wrapping below it
#[test]
fn usage_routing_boundaries_add_past_the_signed_maximum() {
    let deployments = [deployment("dep-a", &[], &[], &[("tpm", number(i64::MAX))])];
    let stored = counts(&[("dep-a", i64::MAX)]);
    let windows = WindowCounts {
        tpm: Some(&stored),
        rpm: None,
    };
    assert_eq!(select_counts(&windows, &deployments, 1).unwrap(), None);
}

#[test]
fn usage_routing_boundaries_keep_the_first_of_two_equal_counts() {
    let deployments = [plain("dep-a"), plain("dep-b")];
    let stored = counts(&[("dep-a", 50), ("dep-b", 50)]);
    let windows = WindowCounts {
        tpm: Some(&stored),
        rpm: None,
    };
    assert_eq!(select_counts(&windows, &deployments, 0).unwrap(), Some(0));
}

// LiteLLM's scan keeps the last healthy entry with an id, so a duplicate id answers with the last deployment
#[test]
fn usage_routing_boundaries_take_the_last_deployment_with_a_duplicate_id() {
    let deployments = [
        deployment("dep-a", &[], &[], &[("tpm", number(0))]),
        deployment("dep-a", &[], &[], &[]),
    ];
    let stored = counts(&[]);
    let windows = WindowCounts {
        tpm: Some(&stored),
        rpm: None,
    };
    assert_eq!(select_counts(&windows, &deployments, 1).unwrap(), Some(1));
}

// the scan follows the window's insertion order, not the pool's, so a tie keeps the first id the window wrote
#[test]
fn usage_routing_boundaries_keep_the_window_insertion_order_on_a_tie() {
    let deployments = [plain("dep-a"), plain("dep-b")];
    let stored = counts(&[("dep-b", 50), ("dep-a", 50)]);
    let windows = WindowCounts {
        tpm: Some(&stored),
        rpm: None,
    };
    assert_eq!(select_counts(&windows, &deployments, 0).unwrap(), Some(1));
}

#[derive(Clone, Copy)]
enum Level {
    Top,
    LitellmParams,
    ModelInfo,
}

impl Level {
    fn deployment(self, field: &'static str, value: Json) -> DeploymentSpec {
        match self {
            Self::Top => deployment("dep-a", &[(field, value)], &[], &[]),
            Self::LitellmParams => deployment("dep-a", &[], &[(field, value)], &[]),
            Self::ModelInfo => deployment("dep-a", &[], &[], &[(field, value)]),
        }
    }
}

#[rstest]
#[case::top_level(Level::Top)]
#[case::litellm_params(Level::LitellmParams)]
#[case::model_info(Level::ModelInfo)]
fn usage_routing_boundaries_name_a_token_limit_that_is_not_a_number(#[case] level: Level) {
    let deployments = [level.deployment("tpm", text("10"))];
    let error = select_counts(&WindowCounts::default(), &deployments, 0).unwrap_err();
    assert!(matches!(error, RoutingError::Metric { field: "tpm" }));
    assert_eq!(error.unbuilt_entry(), None);
}

#[rstest]
#[case::top_level(Level::Top)]
#[case::litellm_params(Level::LitellmParams)]
#[case::model_info(Level::ModelInfo)]
fn usage_routing_boundaries_name_a_request_limit_that_is_not_a_number(#[case] level: Level) {
    let deployments = [level.deployment("rpm", Json::Bool(true))];
    let error = select_counts(&WindowCounts::default(), &deployments, 0).unwrap_err();
    assert!(matches!(error, RoutingError::Metric { field: "rpm" }));
    assert_eq!(error.unbuilt_entry(), None);
}

// an unreadable window is the cold window, and that fallback never hides a limit the request cannot use
#[test]
fn usage_routing_boundaries_never_let_a_store_failure_hide_a_limit_error() {
    let windows = LocalWindows::default();
    windows.fail_reads(true);
    let deployments = [deployment("dep-a", &[], &[], &[("rpm", Json::Bool(true))])];
    let error = select(&windows, GROUP, &deployments, 0, instant(NOW)).unwrap_err();
    assert!(matches!(error, RoutingError::Metric { field: "rpm" }));
}

// the strategy arm answers on its first poll from its arguments alone, so stopping there leaves nothing behind
#[tokio::test]
async fn usage_routing_boundaries_cancel_a_strategy_call_without_state() {
    let deployments = [plain("dep-a"), plain("dep-b")];
    let facts = RequestFacts::default();
    let request = route_request(GROUP, &facts);
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let route = route_context(&request, &services, &limits, &params);
    let mut dispatch = StrategyDispatch::new(RoutingStrategy::UsageBasedRouting, None).unwrap();
    let resolution = Resolution {
        strategy: RoutingStrategy::UsageBasedRouting,
        selector: None,
        source: ResolutionSource::Default,
        bound: false,
        ignored_override: None,
    };
    let mut random = StdRng::seed_from_u64(7);
    {
        let context = strategy_context(&deployments, &route, &mut dispatch, &resolution);
        let mut call = pin!(strategy_select(
            Strategy::UsageBasedRouting,
            context,
            &mut random
        ));
        let first = call.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        assert!(matches!(first, Poll::Ready(Ok(0))));
    }
    let context = strategy_context(&deployments, &route, &mut dispatch, &resolution);
    let selected = strategy_select(Strategy::UsageBasedRouting, context, &mut random).await;
    assert_eq!(selected.unwrap(), 0);
}
