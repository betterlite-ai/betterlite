#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::off_peak::schema::{OffPeakBlock, RateField};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::RecordResponse;
use super::diagnostics::DiagnosticInput;
use crate::cost::prepare::ResponseView;
use crate::helpers::{RESPONSE_COST_HEADER, set_response_cost_in_hidden_params};

mod support;

use support::{
    FakeCatalog, Fakes, INIT, MANAGEMENT, a_price, a_tier, capture, chat_usage, decimal,
    earlier_finding, json_text, object_fields, openai, priced_gpt4o, provider, record, respond,
    state, stored_diagnostic, stored_failure, typed, typed_response,
};

#[test]
fn a_cache_hit_from_the_call_log_returns_zero_before_any_diagnostic() {
    let fakes = Fakes::default();
    let mut log = state(INIT);
    openai(&mut log);
    log.model_call_details
        .insert("cache_hit".to_owned(), Json::Bool(true));
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, Some(Decimal::ZERO));
    assert!(fakes.observe().is_empty());
    assert_eq!(stored_diagnostic(&log), None);
    assert_eq!(stored_failure(&log), None);
}

#[test]
fn an_explicit_cache_flag_beats_the_stored_one() {
    let fakes = Fakes::default().with_cost(decimal("2"));
    let mut log = state(INIT);
    openai(&mut log);
    log.model_call_details
        .insert("cache_hit".to_owned(), Json::Bool(true));
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = respond(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
        Some(false),
    );
    assert_eq!(cost, Some(decimal("2")));
    assert_eq!(fakes.observe().len(), 1);
    assert_eq!(stored_diagnostic(&log), Some(Json::Null));
}

#[test]
fn a_management_read_is_unbilled_but_a_background_response_is_billed() {
    let fakes = Fakes::default().with_cost(decimal("5"));
    let mut unbilled = state(MANAGEMENT);
    openai(&mut unbilled);
    let plain_body = json_text(
        r#"{"model":"gpt-4o","usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
    );
    let cost = record(
        &fakes.recorder(),
        &mut unbilled,
        RecordResponse::Json(&plain_body),
    );
    assert_eq!(cost, Some(Decimal::ZERO));
    assert!(fakes.observe().is_empty());

    let mut billed = state(MANAGEMENT);
    openai(&mut billed);
    let background_body = json_text(
        r#"{"background":true,"model":"gpt-4o","usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
    );
    let billed_cost = record(
        &fakes.recorder(),
        &mut billed,
        RecordResponse::Json(&background_body),
    );
    assert_eq!(billed_cost, Some(decimal("5")));
    assert_eq!(fakes.observe().len(), 1);
}

#[test]
fn the_background_cost_poller_read_stays_billed() {
    let fakes = Fakes::default().with_cost(decimal("5"));
    let mut log = state(
        r#"{"model":"gpt-4o","call_type":"get_responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"metadata":{"internal_call_origin":"background_response_cost_poll"}}}"#,
    );
    openai(&mut log);
    let body = json_text(
        r#"{"model":"gpt-4o","usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
    );
    let cost = record(&fakes.recorder(), &mut log, RecordResponse::Json(&body));
    assert_eq!(cost, Some(decimal("5")));
    assert_eq!(fakes.observe().len(), 1);
}

#[test]
fn the_hidden_response_cost_wins_without_the_additional_charge() {
    let fakes = priced_gpt4o(Fakes::default()).with_cost(decimal("9"));
    let mut log = state(INIT);
    openai(&mut log);
    log.model_call_details
        .insert("additional_response_cost".to_owned(), json_text("0.5"));
    let usage = chat_usage();
    let hidden = object_fields(r#"{"response_cost":0,"model_id":"deployment-2"}"#);
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, Some(Decimal::ZERO));
    assert!(fakes.observe().is_empty());
    assert_eq!(
        stored_diagnostic(&log),
        Some(json_text(
            r#"{"reason":"pricing_not_applied","pricing_model":"gpt-4o","missing_pricing_keys":[]}"#
        ))
    );
    assert!(log.cost.zero_cost_warned);
}

#[test]
fn a_null_hidden_response_cost_is_ignored_and_its_model_id_is_used() {
    let fakes = priced_gpt4o(Fakes::default()).with_cost(decimal("2"));
    let mut log = state(INIT);
    openai(&mut log);
    let usage = chat_usage();
    let hidden = object_fields(r#"{"response_cost":null,"model_id":"deployment-2"}"#);
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, Some(decimal("2")));
    let observed = fakes.observe();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].router_model_id.as_deref(), Some("deployment-2"));
}

#[rstest]
#[case::positive("0.5", "1.75")]
#[case::negative("-1", "1.25")]
#[case::zero("0", "1.25")]
#[case::text("\"1\"", "1.25")]
#[case::boolean_true("true", "2.25")]
#[case::boolean_false("false", "1.25")]
fn additional_response_cost_adds_only_a_positive_number(
    #[case] additional: &str,
    #[case] expected: &str,
) {
    let fakes = Fakes::default().with_cost(decimal("1.25"));
    let mut log = state(INIT);
    log.model_call_details
        .insert("additional_response_cost".to_owned(), json_text(additional));
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, Some(decimal(expected)));
}

#[test]
fn a_failed_calculation_records_the_full_identity_and_the_error_diagnostic() {
    let fakes = priced_gpt4o(Fakes::default()).with_failure();
    let mut log = state(INIT);
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, None);
    assert_eq!(
        stored_failure(&log),
        Some(json_text(
            r#"{"error_str":"boom","traceback_str":"","model":"gpt-4o","cache_hit":false,"custom_llm_provider":"openai","base_model":null,"call_type":"completion","custom_pricing":false}"#
        ))
    );
    assert_eq!(
        stored_diagnostic(&log),
        Some(json_text(
            r#"{"reason":"cost_calculation_error","pricing_model":"gpt-4o","missing_pricing_keys":[]}"#
        ))
    );
    assert!(log.cost.zero_cost_warned);
}

#[test]
fn a_preparation_failure_records_only_the_short_information() {
    let fakes = Fakes::default();
    let mut log = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"base_model":true}}"#,
    );
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, None);
    assert_eq!(
        stored_failure(&log),
        Some(json_text(
            r#"{"error_str":"the base_model value must be a string","traceback_str":""}"#
        ))
    );
    assert_eq!(stored_diagnostic(&log), None);
    assert!(fakes.observe().is_empty());
}

#[test]
fn a_malformed_model_info_returns_a_short_failure_instead_of_escaping() {
    let fakes = Fakes::default();
    let mut log = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"metadata":{"model_info":"deployment"}}}"#,
    );
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, None);
    assert_eq!(
        stored_failure(&log),
        Some(json_text(
            r#"{"error_str":"the router model id could not be read: litellm_params.model_info must be an object","traceback_str":""}"#
        ))
    );
    assert_eq!(stored_diagnostic(&log), None);
    assert!(fakes.observe().is_empty());
}

#[test]
fn a_diagnostic_failure_keeps_the_earlier_finding() {
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_unkeyed_entry(
        "gpt-4o",
        "input_cost_per_image",
        a_price(1),
    ));
    let mut log = state(INIT);
    openai(&mut log);
    log.model_call_details
        .insert("zero_cost_diagnostic".to_owned(), earlier_finding());
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let recorder = fakes.recorder();
    let (cost, warnings) =
        capture(|| record(&recorder, &mut log, typed(typed_response(&usage, &hidden))));
    assert_eq!(cost, Some(Decimal::ZERO));
    assert_eq!(stored_diagnostic(&log), Some(earlier_finding()));
    assert!(warnings.is_empty());
    assert!(!log.cost.zero_cost_warned);
}

#[test]
fn a_nonzero_cost_clears_the_earlier_finding() {
    let fakes = Fakes::default().with_cost(decimal("2"));
    let mut log = state(INIT);
    openai(&mut log);
    log.model_call_details
        .insert("zero_cost_diagnostic".to_owned(), earlier_finding());
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, Some(decimal("2")));
    assert_eq!(stored_diagnostic(&log), Some(Json::Null));
}

#[test]
fn a_null_cost_without_failure_leaves_the_earlier_finding_alone() {
    let fakes = Fakes::default();
    let mut log = state(INIT);
    openai(&mut log);
    log.model_call_details
        .insert("zero_cost_diagnostic".to_owned(), earlier_finding());
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let billing_view = ResponseView::Typed(typed_response(&usage, &hidden));
    fakes.recorder().record_diagnostic(
        &mut log,
        DiagnosticInput {
            view: &billing_view,
            background: false,
            cost: None,
            calculation_failed: false,
            litellm_model_name: None,
            router_model_id: None,
        },
    );
    assert_eq!(stored_diagnostic(&log), Some(earlier_finding()));
}

#[test]
fn an_explicit_stored_cache_hit_clears_the_finding() {
    let fakes = Fakes::default().with_cost(decimal("5"));
    let mut log = state(INIT);
    openai(&mut log);
    log.model_call_details
        .insert("cache_hit".to_owned(), Json::Bool(true));
    log.model_call_details
        .insert("zero_cost_diagnostic".to_owned(), earlier_finding());
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = respond(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
        Some(false),
    );
    assert_eq!(cost, Some(decimal("5")));
    assert_eq!(stored_diagnostic(&log), Some(Json::Null));
    assert!(!log.cost.zero_cost_warned);
}

#[test]
fn a_zero_cost_with_no_pricing_entry_has_no_finding() {
    let fakes = Fakes::default();
    let mut log = state(INIT);
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, Some(Decimal::ZERO));
    assert_eq!(stored_diagnostic(&log), Some(Json::Null));
    assert!(!log.cost.zero_cost_warned);
}

#[test]
fn a_priced_zero_cost_reports_the_missing_keys_and_warns_once() {
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_entry(
        "gpt-4o",
        &[("input_cost_per_image", a_price(1))],
    ));
    let mut log = state(INIT);
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
            r#"{"reason":"missing_pricing_key","pricing_model":"gpt-4o","missing_pricing_keys":["input_cost_per_token","output_cost_per_token"]}"#
        ))
    );
    assert!(log.cost.zero_cost_warned);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("Billable request priced at $0"));
}

#[test]
fn a_tiered_only_entry_reports_the_missing_keys() {
    let tier = a_tier(a_price(1), Some(a_price(1)));
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_tiered("gpt-4o", vec![tier]));
    let mut log = state(INIT);
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
            r#"{"reason":"missing_pricing_key","pricing_model":"gpt-4o","missing_pricing_keys":["input_cost_per_token","output_cost_per_token"]}"#
        ))
    );
    assert!(log.cost.zero_cost_warned);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn a_tiered_only_entry_reports_a_calculation_error() {
    let tier = a_tier(a_price(1), None);
    let fakes = Fakes::default()
        .with_catalog(FakeCatalog::with_tiered("gpt-4o", vec![tier]))
        .with_failure();
    let mut log = state(INIT);
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let cost = record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(cost, None);
    assert_eq!(
        stored_diagnostic(&log),
        Some(json_text(
            r#"{"reason":"cost_calculation_error","pricing_model":"gpt-4o","missing_pricing_keys":[]}"#
        ))
    );
    assert!(log.cost.zero_cost_warned);
}

#[test]
fn a_deployment_model_info_keeps_its_own_rates_instead_of_the_cost_map_tables() {
    let tier = a_tier(a_price(1), Some(a_price(1)));
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_tiered("gpt-4o", vec![tier]));
    let mut log = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"metadata":{"model_info":{"input_cost_per_token":0}}}}"#,
    );
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let recorder = fakes.recorder();
    let (cost, warnings) =
        capture(|| record(&recorder, &mut log, typed(typed_response(&usage, &hidden))));
    assert_eq!(cost, Some(Decimal::ZERO));
    assert_eq!(stored_diagnostic(&log), Some(Json::Null));
    assert!(!log.cost.zero_cost_warned);
    assert_eq!(warnings.len(), 0);
}

#[test]
fn a_router_cost_map_entry_keeps_its_decoded_tables() {
    let tier = a_tier(a_price(1), Some(a_price(1)));
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_tiered("deploy-1", vec![tier]));
    let mut log = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"input_cost_per_token":0,"litellm_metadata":{"model_info":{"id":"deploy-1"}}}}"#,
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
            r#"{"reason":"missing_pricing_key","pricing_model":"deploy-1","missing_pricing_keys":["input_cost_per_token","output_cost_per_token"]}"#
        ))
    );
    assert!(log.cost.zero_cost_warned);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn an_off_peak_only_entry_reports_the_missing_keys() {
    let block = OffPeakBlock {
        hours_utc: None,
        windows: None,
        weekday_timezone: None,
        input_cost_per_token: Some(RateField::Number(Number::from(1_i64))),
        output_cost_per_token: None,
        reasoning_token_cost: None,
        cache_read_input_token_cost: None,
        cache_creation_cost: None,
    };
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_off_peak("gpt-4o", block));
    let mut log = state(INIT);
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
            r#"{"reason":"missing_pricing_key","pricing_model":"gpt-4o","missing_pricing_keys":["input_cost_per_token","output_cost_per_token"]}"#
        ))
    );
    assert!(log.cost.zero_cost_warned);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn the_router_model_id_falls_back_to_metadata() {
    let fakes = Fakes::default();
    let mut log = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"metadata":{"model_info":{"id":"deployment-metadata"}}}}"#,
    );
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    let observed = fakes.observe();
    assert_eq!(
        observed[0].router_model_id.as_deref(),
        Some("deployment-metadata")
    );
}

#[test]
fn the_warning_fires_once_per_call_log() {
    let fakes = Fakes::default().with_catalog(FakeCatalog::with_entry(
        "gpt-4o",
        &[("input_cost_per_image", a_price(1))],
    ));
    let mut log = state(INIT);
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let recorder = fakes.recorder();
    let (_, warnings) = capture(|| {
        record(&recorder, &mut log, typed(typed_response(&usage, &hidden)));
        record(&recorder, &mut log, typed(typed_response(&usage, &hidden)));
    });
    assert_eq!(warnings.len(), 1);
    assert!(log.cost.zero_cost_warned);
}

#[test]
fn custom_pricing_counts_zero_values_and_model_info_off_peak_pricing() {
    let fakes = Fakes::default();
    let usage = chat_usage();
    let hidden = IndexMap::new();

    let mut top_level = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"input_cost_per_token":0}}"#,
    );
    record(
        &fakes.recorder(),
        &mut top_level,
        typed(typed_response(&usage, &hidden)),
    );
    let top_level_observed = fakes.observe();
    assert!(top_level_observed[0].custom_pricing);

    let mut deployment = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"metadata":{"model_info":{"off_peak_pricing":{"peaks":[]}}}}}"#,
    );
    record(
        &fakes.recorder(),
        &mut deployment,
        typed(typed_response(&usage, &hidden)),
    );
    let deployment_observed = fakes.observe();
    assert!(deployment_observed[0].custom_pricing);

    let mut top_level_off_peak = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"off_peak_pricing":{"peaks":[]}}}"#,
    );
    record(
        &fakes.recorder(),
        &mut top_level_off_peak,
        typed(typed_response(&usage, &hidden)),
    );
    let off_peak_observed = fakes.observe();
    assert!(!off_peak_observed[0].custom_pricing);
}

#[test]
fn a_spilled_over_ptu_request_loses_custom_pricing() {
    let fakes = Fakes::default().with_ptu();
    let usage = chat_usage();
    let hidden = IndexMap::new();
    let init = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"input_cost_per_token":0,"metadata":{"model_info":{"ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1","ptu_effective_from":"2026-01-01T00:00:00Z"}}}}"#;

    let mut spilled = state(init);
    spilled.model_call_details.insert(
        "response_headers".to_owned(),
        json_text(r#"{"x-ms-is-spilled-over":"true"}"#),
    );
    record(
        &fakes.recorder(),
        &mut spilled,
        typed(typed_response(&usage, &hidden)),
    );
    let spilled_observed = fakes.observe();
    assert!(!spilled_observed[0].custom_pricing);

    let mut kept = state(init);
    record(
        &fakes.recorder(),
        &mut kept,
        typed(typed_response(&usage, &hidden)),
    );
    let kept_observed = fakes.observe();
    assert!(kept_observed[0].custom_pricing);
}

#[test]
fn vertex_location_prefers_optional_params_then_the_logged_params() {
    let fakes = Fakes::default().with_vertex_location(Some("us-central1"));
    let usage = chat_usage();
    let hidden = IndexMap::new();

    let mut explicit = state(INIT);
    provider(&mut explicit, "vertex_ai");
    explicit.optional_params.insert(
        "vertex_location".to_owned(),
        Json::Text("europe-west1".to_owned()),
    );
    explicit.litellm_params.insert(
        "vertex_ai_location".to_owned(),
        Json::Text("asia-northeast1".to_owned()),
    );
    record(
        &fakes.recorder(),
        &mut explicit,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(fakes.regions(), vec![Some("europe-west1".to_owned())]);
    let explicit_observed = fakes.observe();
    assert_eq!(
        explicit_observed[0].vertex_location.as_deref(),
        Some("normalized-europe-west1")
    );

    let mut logged = state(INIT);
    provider(&mut logged, "vertex_ai");
    logged.litellm_params.insert(
        "vertex_ai_location".to_owned(),
        Json::Text("asia-northeast1".to_owned()),
    );
    record(
        &fakes.recorder(),
        &mut logged,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(fakes.regions(), vec![Some("asia-northeast1".to_owned())]);
}

#[test]
fn vertex_location_defaults_only_for_vertex_ai_providers() {
    let fakes = Fakes::default().with_vertex_location(Some("us-central1"));
    let usage = chat_usage();
    let hidden = IndexMap::new();

    let mut defaulted = state(INIT);
    provider(&mut defaulted, "vertex_ai_beta");
    record(
        &fakes.recorder(),
        &mut defaulted,
        typed(typed_response(&usage, &hidden)),
    );
    assert_eq!(fakes.regions(), vec![Some("us-central1".to_owned())]);

    let mut other = state(INIT);
    openai(&mut other);
    record(
        &fakes.recorder(),
        &mut other,
        typed(typed_response(&usage, &hidden)),
    );
    assert!(fakes.regions().is_empty());
    let other_observed = fakes.observe();
    assert_eq!(other_observed[0].vertex_location, None);
}

#[test]
fn the_mantle_region_is_resolved_only_for_bedrock_mantle() {
    let fakes = Fakes::default();
    let usage = chat_usage();
    let hidden = IndexMap::new();

    let mut mantle = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"aws_region_name":"eu-central-1"}}"#,
    );
    provider(&mut mantle, "bedrock_mantle");
    record(
        &fakes.recorder(),
        &mut mantle,
        typed(typed_response(&usage, &hidden)),
    );
    let calls = fakes.mantle_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].get("aws_region_name"),
        Some(&Json::Text("eu-central-1".to_owned()))
    );
    let mantle_observed = fakes.observe();
    assert_eq!(mantle_observed[0].region_name.as_deref(), Some("us-east-1"));

    let mut other = state(INIT);
    openai(&mut other);
    record(
        &fakes.recorder(),
        &mut other,
        typed(typed_response(&usage, &hidden)),
    );
    assert!(fakes.mantle_calls().is_empty());
    let other_observed = fakes.observe();
    assert_eq!(other_observed[0].region_name, None);
}

#[test]
fn only_speech_prices_the_input_string() {
    let fakes = Fakes::default();
    let usage = chat_usage();
    let hidden = IndexMap::new();

    let mut speech = state(
        r#"{"model":"gpt-4o","call_type":"speech","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
    );
    openai(&mut speech);
    speech
        .model_call_details
        .insert("input".to_owned(), Json::Text("hello".to_owned()));
    record(
        &fakes.recorder(),
        &mut speech,
        typed(typed_response(&usage, &hidden)),
    );
    let speech_observed = fakes.observe();
    assert_eq!(speech_observed[0].prompt, "hello");

    let mut completion = state(INIT);
    openai(&mut completion);
    completion
        .model_call_details
        .insert("input".to_owned(), Json::Text("hello".to_owned()));
    record(
        &fakes.recorder(),
        &mut completion,
        typed(typed_response(&usage, &hidden)),
    );
    let completion_observed = fakes.observe();
    assert_eq!(completion_observed[0].prompt, "");
}

#[test]
fn generate_content_uses_the_converted_response_and_falls_back_on_failure() {
    let fakes = Fakes::default().with_conversion(Some(json_text(
        r#"{"model":"gpt-4o","usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#,
    )));
    let native = json_text(
        r#"{"candidates":[{"content":{}}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}"#,
    );
    let mut converted = state(
        r#"{"model":"gpt-4o","call_type":"generate_content","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
    );
    openai(&mut converted);
    record(
        &fakes.recorder(),
        &mut converted,
        RecordResponse::GenerateContent(&native),
    );
    let converted_observed = fakes.observe();
    assert_eq!(converted_observed[0].usage, Some((7, 3)));

    let failing = Fakes::default().with_conversion(None);
    let mut original = state(
        r#"{"model":"gpt-4o","call_type":"generate_content","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
    );
    openai(&mut original);
    let body = json_text(
        r#"{"model":"gpt-4o","usage":{"prompt_tokens":4,"completion_tokens":2,"total_tokens":6}}"#,
    );
    record(
        &failing.recorder(),
        &mut original,
        RecordResponse::GenerateContent(&body),
    );
    let fallback_observed = failing.observe();
    assert_eq!(fallback_observed[0].usage, Some((4, 2)));
}

#[test]
fn a_terminal_event_is_unwrapped_and_an_earlier_event_is_not() {
    let fakes = Fakes::default();
    let terminal = json_text(
        r#"{"type":"response.completed","response":{"model":"gpt-4o","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#,
    );
    let mut completed = state(INIT);
    openai(&mut completed);
    record(
        &fakes.recorder(),
        &mut completed,
        RecordResponse::Event(&terminal),
    );
    let terminal_observed = fakes.observe();
    assert_eq!(terminal_observed[0].usage, Some((10, 5)));

    let created = json_text(
        r#"{"type":"response.created","response":{"model":"gpt-4o","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#,
    );
    let mut early = state(INIT);
    openai(&mut early);
    record(
        &fakes.recorder(),
        &mut early,
        RecordResponse::Event(&created),
    );
    let early_observed = fakes.observe();
    assert_eq!(early_observed.len(), 1);
    assert_eq!(early_observed[0].usage, None);
}

#[test]
fn the_router_model_id_prefers_litellm_metadata() {
    let fakes = Fakes::default();
    let mut log = state(
        r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_params":{"metadata":{"model_info":{"id":"deployment-metadata"}},"litellm_metadata":{"model_info":{"id":"deployment-litellm"}}}}"#,
    );
    openai(&mut log);
    let usage = chat_usage();
    let hidden = IndexMap::new();
    record(
        &fakes.recorder(),
        &mut log,
        typed(typed_response(&usage, &hidden)),
    );
    let observed = fakes.observe();
    assert_eq!(
        observed[0].router_model_id.as_deref(),
        Some("deployment-litellm")
    );
}

#[test]
fn the_cost_header_merge_keeps_existing_headers_and_adds_no_response_cost() {
    let mut hidden = object_fields(r#"{"additional_headers":{"x-custom":"kept"}}"#);
    set_response_cost_in_hidden_params(&mut hidden, Some(decimal("0.25")));
    assert_eq!(
        hidden.get("additional_headers"),
        Some(&json_text(
            r#"{"x-custom":"kept","llm_provider-x-litellm-response-cost":0.25}"#
        ))
    );
    assert_eq!(hidden.get("response_cost"), None);
    assert_eq!(RESPONSE_COST_HEADER, "llm_provider-x-litellm-response-cost");
}

#[test]
fn a_null_cost_leaves_the_hidden_params_untouched() {
    let mut hidden = object_fields(r#"{"additional_headers":{"x-custom":"kept"}}"#);
    let before = hidden.clone();
    set_response_cost_in_hidden_params(&mut hidden, None);
    assert_eq!(hidden, before);
}

#[test]
fn a_null_additional_headers_is_replaced() {
    let mut hidden = object_fields(r#"{"additional_headers":null}"#);
    set_response_cost_in_hidden_params(&mut hidden, Some(Decimal::ZERO));
    assert_eq!(
        hidden.get("additional_headers"),
        Some(&json_text(r#"{"llm_provider-x-litellm-response-cost":0}"#))
    );
}
