#![cfg(test)]

use rstest::rstest;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_aware_routing::tests::support::DEPLOYMENT_ID;
use crate::cache_aware_routing::tests::support::FakeCost;
use crate::cache_aware_routing::tests::support::FakeMinimum;
use crate::cache_aware_routing::tests::support::LimitsLookup;
use crate::cache_aware_routing::tests::support::MemoryCache;
use crate::cache_aware_routing::tests::support::PARAMS;
use crate::cache_aware_routing::tests::support::Resolution;
use crate::cache_aware_routing::tests::support::model_info;
use crate::cache_aware_routing::tests::support::params;
use crate::cache_aware_routing::tests::support::prediction_services;
use crate::cache_aware_routing::tests::support::settings;

fn fits(
    params_json: &str,
    info_json: &str,
    input_tokens: i64,
    output_tokens: i64,
    lookup: &LimitsLookup,
) -> bool {
    let litellm_params = params(params_json);
    let info = params(info_json);
    let deployment = PredictionDeployment::read(&litellm_params, &info);
    let resolution = Resolution::new();
    let context = resolution.context();
    let settings = settings("https://api.anthropic.com");
    let cache = MemoryCache::new();
    let cost = FakeCost::new(None);
    let minimum = FakeMinimum { tokens: 0 };
    let services = prediction_services(&settings, &context, &cache, &cost, &minimum);
    AnthropicCacheRouting::fits(&deployment, input_tokens, output_tokens, &services, lookup)
}

#[rstest]
#[case::comfortably_inside(100, 10)]
#[case::exactly_at_both_limits(60, 40)]
fn a_request_inside_the_declared_limits_fits(#[case] input: i64, #[case] output: i64) {
    let info = r#"{"id":"deployment-a","max_input_tokens":100,"max_output_tokens":50}"#;
    assert!(fits(
        PARAMS,
        info,
        input,
        output,
        &LimitsLookup {
            max_input: None,
            max_output: None,
        }
    ));
}

#[rstest]
#[case::input_window(61, 40)]
#[case::output_limit(10, 51)]
fn a_request_outside_one_declared_limit_does_not_fit(#[case] input: i64, #[case] output: i64) {
    let info = r#"{"id":"deployment-a","max_input_tokens":100,"max_output_tokens":50}"#;
    assert!(!fits(
        PARAMS,
        info,
        input,
        output,
        &LimitsLookup {
            max_input: None,
            max_output: None,
        }
    ));
}

#[test]
fn the_catalog_limits_stand_in_for_the_deployment_ones() {
    let info = r#"{"id":"deployment-a"}"#;
    let lookup = LimitsLookup {
        max_input: Some(100),
        max_output: Some(50),
    };
    assert!(fits(PARAMS, info, 60, 40, &lookup));
    assert!(!fits(PARAMS, info, 61, 40, &lookup));
    assert!(!fits(PARAMS, info, 60, 51, &lookup));
}

#[test]
fn the_declared_limits_win_over_the_catalog_ones() {
    let info = r#"{"id":"deployment-a","max_input_tokens":100,"max_output_tokens":50}"#;
    let lookup = LimitsLookup {
        max_input: Some(10),
        max_output: Some(5),
    };
    assert!(fits(PARAMS, info, 60, 40, &lookup));
}

#[rstest]
#[case::nothing_declared(r#"{"id":"deployment-a"}"#, LimitsLookup { max_input: None, max_output: None })]
#[case::only_an_input_limit(r#"{"id":"deployment-a","max_input_tokens":100}"#, LimitsLookup { max_input: None, max_output: None })]
#[case::only_an_output_limit(r#"{"id":"deployment-a","max_output_tokens":50}"#, LimitsLookup { max_input: None, max_output: None })]
#[case::a_text_limit(r#"{"id":"deployment-a","max_input_tokens":"100","max_output_tokens":"50"}"#, LimitsLookup { max_input: None, max_output: None })]
fn a_request_without_both_limits_does_not_fit(#[case] info: &str, #[case] lookup: LimitsLookup) {
    assert!(!fits(PARAMS, info, 1, 1, &lookup));
}

#[test]
fn a_token_count_that_overflows_does_not_fit() {
    assert!(!fits(
        PARAMS,
        &format!(r#"{{"id":"{DEPLOYMENT_ID}","max_input_tokens":100,"max_output_tokens":50}}"#),
        i64::MAX,
        1,
        &LimitsLookup {
            max_input: None,
            max_output: None,
        }
    ));
}

#[test]
fn an_unpredictable_deployment_does_not_fit() {
    let params_json =
        r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","organization":"org"}"#;
    assert!(!fits(
        params_json,
        &format!(r#"{{"id":"{DEPLOYMENT_ID}","max_input_tokens":100,"max_output_tokens":50}}"#),
        1,
        1,
        &LimitsLookup {
            max_input: None,
            max_output: None,
        }
    ));
}

#[test]
fn the_fixture_deployment_fits_its_own_catalog_limits() {
    let info = model_info();
    let text = serde_json::to_string(&info).expect("model info");
    assert!(fits(
        PARAMS,
        &text,
        100,
        10,
        &LimitsLookup {
            max_input: None,
            max_output: None
        }
    ));
}
