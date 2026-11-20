use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::super::RecordInput;
use super::support::{
    FakeCatalog, Fakes, INIT, a_price, capture, chat_usage, json_text, openai, record, run, state,
    stored_diagnostic, typed, typed_response,
};

const ROUTER_METADATA: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"metadata":{"model_info":{"id":"deployment-metadata","input_cost_per_token":1}}}}"#;

#[rstest]
#[case::explicit_id("explicit-id", "explicit-id")]
#[case::empty_falls_back("", "deployment-metadata")]
fn an_explicit_router_id_wins_and_an_empty_one_falls_back(
    #[case] explicit: &str,
    #[case] expected: &str,
) {
    let fakes = Fakes::default();
    let mut log = state(ROUTER_METADATA);
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = run(fakes.recorder().record(
        &mut log,
        RecordInput {
            response: typed(typed_response(&usage, &hidden)),
            cache_hit: None,
            litellm_model_name: None,
            router_model_id: Some(explicit),
        },
    ));
    assert_eq!(cost, Some(Decimal::ZERO));
    let observed = fakes.observe();
    assert_eq!(observed[0].router_model_id.as_deref(), Some(expected));
    assert_eq!(
        stored_diagnostic(&log),
        Some(json_text(&format!(
            r#"{{"reason":"missing_pricing_key","pricing_model":"{expected}","missing_pricing_keys":["output_cost_per_token"]}}"#
        )))
    );
}

#[rstest]
#[case::named_model("named-model", "named-model")]
#[case::empty_falls_back("", "gpt-4o")]
fn an_explicit_model_name_wins_and_an_empty_one_falls_back(
    #[case] named: &str,
    #[case] expected: &str,
) {
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_entry(
        "gpt-4o",
        &[("input_cost_per_image", a_price(1))],
    ));
    let mut log = state(INIT);
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let recorder = fakes.recorder();
    let (cost, warnings) = capture(|| {
        run(recorder.record(
            &mut log,
            RecordInput {
                response: typed(typed_response(&usage, &hidden)),
                cache_hit: None,
                litellm_model_name: Some(named),
                router_model_id: None,
            },
        ))
    });
    assert_eq!(cost, Some(Decimal::ZERO));
    let observed = fakes.observe();
    assert_eq!(observed[0].model.as_str(), expected);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains(format!("model={expected}").as_str()));
}

#[test]
fn a_router_entry_with_a_foreign_key_ignores_the_deployment_model_info() {
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_keyed_entry(
        "deploy-1",
        "actual-model",
        &[
            ("input_cost_per_token", a_price(1)),
            ("output_cost_per_token", a_price(1)),
        ],
    ));
    let mut log = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"input_cost_per_token":0,"metadata":{"model_info":{"base_model":"base"}},"litellm_metadata":{"model_info":{"id":"deploy-1"}}}}"#,
    );
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let recorder = fakes.recorder();
    let (cost, warnings) =
        capture(|| record(&recorder, &mut log, typed(typed_response(&usage, &hidden))));
    assert_eq!(cost, Some(Decimal::ZERO));
    assert_eq!(
        stored_diagnostic(&log),
        Some(json_text(
            r#"{"reason":"pricing_not_applied","pricing_model":"deploy-1","missing_pricing_keys":[]}"#
        ))
    );
    assert!(log.cost.zero_cost_warned);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn an_audio_usage_reports_the_missing_audio_keys() {
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_entry(
        "gpt-4o",
        &[
            ("input_cost_per_token", a_price(1)),
            ("output_cost_per_token", a_price(1)),
        ],
    ));
    let mut log = state(INIT);
    openai(&mut log);
    let mut usage = chat_usage();
    usage.prompt_tokens_details = Some(IndexMap::from([(
        "audio_tokens".to_owned(),
        Json::Number(Number::from(3)),
    )]));
    usage.completion_tokens_details = Some(IndexMap::from([(
        "audio_tokens".to_owned(),
        Json::Number(Number::from(2)),
    )]));
    let hidden = IndexMap::new();
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, Some(Decimal::ZERO));
    assert_eq!(
        stored_diagnostic(&log),
        Some(json_text(
            r#"{"reason":"missing_pricing_key","pricing_model":"gpt-4o","missing_pricing_keys":["input_cost_per_audio_token","output_cost_per_audio_token"]}"#
        ))
    );
    assert!(log.cost.zero_cost_warned);
}
