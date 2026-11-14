#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::HeaderMap;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::detailed_timing::DetailedTiming;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::json;

use super::custom_headers;
use super::input::{CustomHeadersInput, KeyHeaderView, float_text};

fn build(input: JsonValue) -> HeaderMap {
    build_with_timing(input, false)
}

fn build_with_timing(input: JsonValue, enabled: bool) -> HeaderMap {
    let parsed: CustomHeadersInput = serde_json::from_value(input).expect("the input parses");
    custom_headers(&parsed, DetailedTiming { enabled })
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| value.to_str().expect("a text header").to_owned())
}

#[rstest]
#[case::given_first(
    json!({"call_id": "call-given", "hidden_params": {"litellm_call_id": "hidden"}, "request_data": {"litellm_call_id": "request"}}),
    Some("call-given")
)]
#[case::hidden_before_request(
    json!({"hidden_params": {"litellm_call_id": "hidden"}, "request_data": {"litellm_call_id": "request"}}),
    Some("hidden")
)]
#[case::empty_given_falls_through(
    json!({"call_id": "", "hidden_params": {"litellm_call_id": "hidden"}}),
    Some("hidden")
)]
#[case::empty_hidden_falls_through(
    json!({"hidden_params": {"litellm_call_id": ""}, "request_data": {"litellm_call_id": "request"}}),
    Some("request")
)]
#[case::empty_array_hidden_falls_through(
    json!({"hidden_params": {"litellm_call_id": []}, "request_data": {"litellm_call_id": "request"}}),
    Some("request")
)]
#[case::empty_object_hidden_falls_through(
    json!({"hidden_params": {"litellm_call_id": {}}, "request_data": {"litellm_call_id": "request"}}),
    Some("request")
)]
#[case::number_is_text(json!({"hidden_params": {"litellm_call_id": 7}}), Some("7"))]
#[case::nowhere(json!({}), None)]
fn call_id_comes_from_the_first_source(#[case] input: JsonValue, #[case] expected: Option<&str>) {
    let headers = build(input);
    assert_eq!(header(&headers, "x-litellm-call-id").as_deref(), expected);
}

#[test]
fn identity_headers_carry_the_given_values() {
    let headers = build(json!({
        "model_id": "model-1",
        "cache_key": "key-1",
        "api_base": "https://api.example/v1",
        "version": "1.2.3",
        "model_region": "eu"
    }));
    assert_eq!(
        header(&headers, "x-litellm-model-id").as_deref(),
        Some("model-1")
    );
    assert_eq!(
        header(&headers, "x-litellm-cache-key").as_deref(),
        Some("key-1")
    );
    assert_eq!(
        header(&headers, "x-litellm-model-api-base").as_deref(),
        Some("https://api.example/v1")
    );
    assert_eq!(
        header(&headers, "x-litellm-version").as_deref(),
        Some("1.2.3")
    );
    assert_eq!(
        header(&headers, "x-litellm-model-region").as_deref(),
        Some("eu")
    );
}

#[rstest]
#[case::query_dropped(json!("https://host/v1?key=secret"), Some("https://host/v1"))]
#[case::plain(json!("https://host/v1"), Some("https://host/v1"))]
#[case::empty(json!(""), None)]
#[case::absent(json!(null), None)]
fn api_base_drops_the_query_string(#[case] api_base: JsonValue, #[case] expected: Option<&str>) {
    let headers = build(json!({"api_base": api_base}));
    assert_eq!(
        header(&headers, "x-litellm-model-api-base").as_deref(),
        expected
    );
}

#[rstest]
#[case::hidden_wins(
    json!({
        "hidden_params": {"_response_ms": 12.5, "litellm_overhead_time_ms": 2.5},
        "logging": {"response_timing_metrics": {"_response_ms": 99.0, "litellm_overhead_time_ms": 88.0}}
    }),
    Some("12.5"),
    Some("2.5")
)]
#[case::null_hidden_falls_to_logging(
    json!({
        "hidden_params": {"_response_ms": null, "litellm_overhead_time_ms": 5.0},
        "logging": {"response_timing_metrics": {"_response_ms": 7.0, "litellm_overhead_time_ms": 1.0}}
    }),
    Some("7.0"),
    Some("1.0")
)]
#[case::flag_off_keeps_hidden(
    json!({
        "read_timing_from_logging": false,
        "hidden_params": {"_response_ms": 3.0, "litellm_overhead_time_ms": 4.0},
        "logging": {"response_timing_metrics": {"_response_ms": 7.0}}
    }),
    Some("3.0"),
    Some("4.0")
)]
#[case::flag_off_without_hidden_response_ms(
    json!({
        "read_timing_from_logging": false,
        "hidden_params": {"litellm_overhead_time_ms": 4.0},
        "logging": {"response_timing_metrics": {"_response_ms": 7.0, "litellm_overhead_time_ms": 1.0}}
    }),
    None,
    Some("4.0")
)]
#[case::no_logging_view(
    json!({"hidden_params": {"_response_ms": 3.0, "litellm_overhead_time_ms": 4.0}}),
    Some("3.0"),
    Some("4.0")
)]
#[case::logging_without_metrics(
    json!({"logging": {"call_type": "completion"}}),
    None,
    None
)]
fn timing_headers_come_from_one_source(
    #[case] input: JsonValue,
    #[case] response: Option<&str>,
    #[case] overhead: Option<&str>,
) {
    let headers = build(input);
    assert_eq!(
        header(&headers, "x-litellm-response-duration-ms").as_deref(),
        response
    );
    assert_eq!(
        header(&headers, "x-litellm-overhead-duration-ms").as_deref(),
        overhead
    );
}

#[test]
fn callback_duration_comes_from_the_hidden_params() {
    let headers = build(json!({"hidden_params": {"callback_duration_ms": 9}}));
    assert_eq!(
        header(&headers, "x-litellm-callback-duration-ms").as_deref(),
        Some("9")
    );
}

#[test]
fn without_a_logging_view_no_breakdown_header_appears() {
    let headers = build(json!({"response_cost": 1.5}));
    assert_eq!(
        header(&headers, "x-litellm-response-cost").as_deref(),
        Some("1.5")
    );
    assert_eq!(header(&headers, "x-litellm-response-cost-original"), None);
    assert_eq!(header(&headers, "x-litellm-response-cost-input"), None);
}

#[test]
fn an_empty_breakdown_leaves_the_part_headers_out() {
    let headers = build(json!({"logging": {"call_type": "completion", "cost_breakdown": {}}}));
    assert_eq!(header(&headers, "x-litellm-response-cost-original"), None);
    assert_eq!(header(&headers, "x-litellm-response-cost-tool-usage"), None);
}

#[test]
fn an_empty_breakdown_still_zero_splits_a_non_inference_call() {
    let headers = build(json!({
        "response_cost": 0.0,
        "logging": {"call_type": "get_responses", "cost_breakdown": {}}
    }));
    assert_eq!(
        header(&headers, "x-litellm-response-cost-original").as_deref(),
        Some("0.0")
    );
}

#[rstest]
#[case::non_inference_at_zero("get_responses", json!(0.0), true)]
#[case::non_inference_unpriced("get_responses", json!(null), true)]
#[case::non_inference_empty_string("get_responses", json!(""), true)]
#[case::non_inference_zero_string("get_responses", json!("0"), true)]
#[case::inference_at_zero("completion", json!(0.0), false)]
#[case::non_inference_priced("get_responses", json!(2.5), false)]
#[case::non_inference_unreadable("get_responses", json!("abc"), false)]
fn zero_split_only_for_an_unpriced_non_inference_call(
    #[case] call_type: &str,
    #[case] response_cost: JsonValue,
    #[case] zero: bool,
) {
    let headers = build(json!({
        "response_cost": response_cost,
        "logging": {"call_type": call_type, "cost_breakdown": null}
    }));
    let expected = if zero { Some("0.0") } else { None };
    assert_eq!(
        header(&headers, "x-litellm-response-cost-original").as_deref(),
        expected
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-margin-amount").as_deref(),
        expected
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-input").as_deref(),
        expected
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-tool-usage").as_deref(),
        expected
    );
    assert_eq!(header(&headers, "x-litellm-response-cost-cache-read"), None);
    assert_eq!(
        header(&headers, "x-litellm-response-cost-cache-creation"),
        None
    );
    assert_eq!(header(&headers, "x-litellm-response-cost-reasoning"), None);
}

#[test]
fn breakdown_headers_read_every_part_with_the_uncached_input_cost() {
    let headers = build(json!({
        "logging": {
            "call_type": "completion",
            "cost_breakdown": {
                "original_cost": 2,
                "discount_amount": 0.5,
                "margin_total_amount": 0.25,
                "margin_percent": 0.2,
                "input_cost": 1.0,
                "cache_read_cost": 0.25,
                "cache_creation_cost": 0.5,
                "output_cost": 0.75,
                "tool_usage_cost": 0.125
            }
        }
    }));
    assert_eq!(
        header(&headers, "x-litellm-response-cost-original").as_deref(),
        Some("2")
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-discount-amount").as_deref(),
        Some("0.5")
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-margin-amount").as_deref(),
        Some("0.25")
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-margin-percent").as_deref(),
        Some("0.2")
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-input").as_deref(),
        Some("0.25")
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-output").as_deref(),
        Some("0.75")
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-cache-read").as_deref(),
        Some("0.25")
    );
    assert_eq!(
        header(&headers, "x-litellm-response-cost-cache-creation").as_deref(),
        Some("0.5")
    );
    assert_eq!(header(&headers, "x-litellm-response-cost-reasoning"), None);
    assert_eq!(
        header(&headers, "x-litellm-response-cost-tool-usage").as_deref(),
        Some("0.125")
    );
}

#[test]
fn a_null_cache_part_counts_as_zero_and_a_missing_input_cost_leaves_the_header_out() {
    let counted = build(json!({
        "logging": {"call_type": "completion", "cost_breakdown": {"input_cost": 1.0, "cache_read_cost": null}}
    }));
    assert_eq!(
        header(&counted, "x-litellm-response-cost-input").as_deref(),
        Some("1.0")
    );
    assert_eq!(header(&counted, "x-litellm-response-cost-cache-read"), None);
    let missing = build(json!({
        "logging": {"call_type": "completion", "cost_breakdown": {"cache_read_cost": 0.1}}
    }));
    assert_eq!(header(&missing, "x-litellm-response-cost-input"), None);
}

#[test]
fn a_non_numeric_input_cost_leaves_the_header_out_and_other_parts_still_print() {
    let headers = build(json!({
        "logging": {"call_type": "completion", "cost_breakdown": {"input_cost": "0.3", "original_cost": "0.5"}}
    }));
    assert_eq!(header(&headers, "x-litellm-response-cost-input"), None);
    assert_eq!(
        header(&headers, "x-litellm-response-cost-original").as_deref(),
        Some("0.5")
    );
}

#[test]
fn falsey_cache_parts_count_as_zero_and_a_bool_input_cost_is_a_number() {
    let falsey = build(json!({
        "logging": {"call_type": "completion", "cost_breakdown": {
            "input_cost": 1.0, "cache_read_cost": false, "cache_creation_cost": ""
        }}
    }));
    assert_eq!(
        header(&falsey, "x-litellm-response-cost-input").as_deref(),
        Some("1.0")
    );
    let boolean = build(json!({
        "logging": {"call_type": "completion", "cost_breakdown": {
            "input_cost": true, "cache_read_cost": true
        }}
    }));
    assert_eq!(
        header(&boolean, "x-litellm-response-cost-input").as_deref(),
        Some("0.0")
    );
}

#[test]
fn a_non_numeric_cache_part_leaves_the_input_cost_header_out() {
    let headers = build(json!({
        "logging": {"call_type": "completion", "cost_breakdown": {"input_cost": 1.0, "cache_read_cost": "0.5"}}
    }));
    assert_eq!(header(&headers, "x-litellm-response-cost-input"), None);
    assert_eq!(
        header(&headers, "x-litellm-response-cost-cache-read").as_deref(),
        Some("0.5")
    );
}

#[rstest]
#[case::fraction(12.5, "12.5")]
#[case::zero(0.0, "0.0")]
#[case::whole(2.0, "2.0")]
#[case::smallest_fixed(0.0001, "0.0001")]
#[case::smallest_scientific(0.00001, "1e-05")]
#[case::largest_fixed(1e15, "1000000000000000.0")]
#[case::smallest_large_scientific(1e16, "1e+16")]
#[case::fraction_scientific(1.5e16, "1.5e+16")]
#[case::negative_scientific(-1e-7, "-1e-07")]
#[case::not_a_number(f64::NAN, "nan")]
#[case::infinite(f64::INFINITY, "inf")]
#[case::negative_infinite(f64::NEG_INFINITY, "-inf")]
fn float_text_matches_python_str(#[case] value: f64, #[case] expected: &str) {
    assert_eq!(float_text(value), expected);
}

#[rstest]
#[case::number(json!(1.5), Some("3.5"))]
#[case::string(json!("0.25"), Some("2.25"))]
#[case::padded_string(json!(" 0.25 "), Some("2.25"))]
#[case::non_ascii_digits(json!("\u{0661}\u{0660}"), Some("2.0"))]
#[case::bad_string(json!("oops"), Some("2.0"))]
#[case::underscore_string(json!("1_0"), Some("12.0"))]
#[case::underscore_fraction(json!("1_0.5"), Some("12.5"))]
#[case::leading_underscore(json!("_1"), Some("2.0"))]
#[case::trailing_underscore(json!("1_"), Some("2.0"))]
#[case::doubled_underscore(json!("1__0"), Some("2.0"))]
#[case::underscore_before_point(json!("1_.5"), Some("2.0"))]
#[case::underscore_after_point(json!("1._5"), Some("2.0"))]
#[case::negative(json!(-1.0), Some("2.0"))]
#[case::zero(json!(0), Some("2.0"))]
#[case::absent(json!(null), Some("2.0"))]
fn spend_adds_only_a_positive_response_cost(
    #[case] response_cost: JsonValue,
    #[case] expected: Option<&str>,
) {
    let headers = build(json!({"key": {"spend": 2.0}, "response_cost": response_cost}));
    assert_eq!(header(&headers, "x-litellm-key-spend").as_deref(), expected);
}

#[test]
fn spend_starts_at_zero_when_the_key_has_none() {
    let headers = build(json!({"response_cost": 1.5}));
    assert_eq!(
        header(&headers, "x-litellm-key-spend").as_deref(),
        Some("1.5")
    );
}

#[test]
fn key_limits_come_from_the_serialized_key() {
    let key_fields: BTreeMap<String, JsonData> = serde_json::from_str(
        r#"{"tpm_limit": 100, "rpm_limit": 10, "max_budget": 25.5, "spend": 1.5, "allowed_model_region": "eu"}"#,
    )
    .expect("the key fixture parses");
    let key =
        UserAPIKeyAuth::construct(AuthInput::Dict(key_fields)).expect("the key fixture constructs");
    let view = KeyHeaderView::from_key(&key).expect("the key's header fields are read");
    assert_eq!(view.tpm_limit, Some(100));
    assert_eq!(view.rpm_limit, Some(10));
    assert_eq!(view.max_budget, Some(Decimal::new(255, 1)));
    assert_eq!(view.spend, Some(Decimal::new(15, 1)));
    assert_eq!(view.allowed_model_region.as_deref(), Some("eu"));
}

#[rstest]
#[case::litellm_metadata_first(
    json!({"logging": {"litellm_params": {
        "litellm_metadata": {"deployment": "azure/gpt-4o"},
        "metadata": {"deployment": "openai/gpt-4o"}
    }}}),
    Some("azure/gpt-4o")
)]
#[case::metadata_when_litellm_metadata_is_empty(
    json!({"logging": {"litellm_params": {
        "litellm_metadata": {"deployment": ""},
        "metadata": {"deployment": "openai/gpt-4o"}
    }}}),
    Some("openai/gpt-4o")
)]
#[case::a_text_bucket_is_skipped(
    json!({"logging": {"litellm_params": {
        "litellm_metadata": "text",
        "metadata": {"deployment": "openai/gpt-4o"}
    }}}),
    Some("openai/gpt-4o")
)]
#[case::nowhere(json!({"logging": {"litellm_params": {"metadata": {}}}}), None)]
#[case::no_params(json!({"logging": {}}), None)]
fn deployment_name_reads_both_buckets(#[case] input: JsonValue, #[case] expected: Option<&str>) {
    let headers = build(input);
    assert_eq!(
        header(&headers, "x-litellm-model-name").as_deref(),
        expected
    );
}

#[rstest]
#[case::litellm_metadata_first(
    json!({
        "litellm_metadata": {"routing_decision": {"classifier_cost": 0.0012}},
        "metadata": {"routing_decision": {"classifier_cost": 5}}
    }),
    Some("0.0012")
)]
#[case::metadata_when_the_first_holds_none(
    json!({
        "litellm_metadata": {"routing_decision": {}},
        "metadata": {"routing_decision": {"classifier_cost": 2}}
    }),
    Some("2.0")
)]
#[case::non_numeric_costs_are_skipped(
    json!({
        "litellm_metadata": {"routing_decision": {"classifier_cost": "0.5"}},
        "metadata": {"routing_decision": {"classifier_cost": true}}
    }),
    None
)]
#[case::absent(json!({}), None)]
fn classifier_cost_reads_both_buckets(
    #[case] request_data: JsonValue,
    #[case] expected: Option<&str>,
) {
    let headers = build(json!({"request_data": request_data}));
    assert_eq!(
        header(&headers, "x-litellm-classifier-cost").as_deref(),
        expected
    );
}

#[test]
fn detailed_timing_headers_appear_only_when_the_flag_is_on() {
    let input = json!({"hidden_params": {
        "timing_pre_processing_ms": 1.0,
        "timing_llm_api_ms": 2.0,
        "timing_post_processing_ms": 3.0,
        "timing_message_copy_ms": 4.0
    }});
    let off = build_with_timing(input.clone(), false);
    assert_eq!(header(&off, "x-litellm-timing-llm-api-ms"), None);
    let on = build_with_timing(input, true);
    assert_eq!(
        header(&on, "x-litellm-timing-pre-processing-ms").as_deref(),
        Some("1.0")
    );
    assert_eq!(
        header(&on, "x-litellm-timing-llm-api-ms").as_deref(),
        Some("2.0")
    );
    assert_eq!(
        header(&on, "x-litellm-timing-post-processing-ms").as_deref(),
        Some("3.0")
    );
    assert_eq!(
        header(&on, "x-litellm-timing-message-copy-ms").as_deref(),
        Some("4.0")
    );
}

#[test]
fn extra_pairs_replace_the_headers_before_them() {
    let headers = build(json!({
        "model_id": "fixed",
        "extra": [
            ["x-litellm-model-id", "replaced"],
            ["x-litellm-cache-key", ""],
            ["x-custom", false]
        ]
    }));
    assert_eq!(
        header(&headers, "x-litellm-model-id").as_deref(),
        Some("replaced")
    );
    assert_eq!(header(&headers, "x-litellm-cache-key"), None);
    assert_eq!(header(&headers, "x-custom").as_deref(), Some("False"));
    let cleared = build(json!({
        "cache_key": "key-1",
        "extra": [["x-litellm-cache-key", "new"], ["x-litellm-cache-key", null]]
    }));
    assert_eq!(header(&cleared, "x-litellm-cache-key"), None);
}

#[rstest]
#[case::true_value(json!(true), "True")]
#[case::false_value(json!(false), "False")]
#[case::absent(json!(null), "")]
fn fastest_batch_completion_is_python_text(#[case] value: JsonValue, #[case] expected: &str) {
    let headers = build(json!({"fastest_response_batch_completion": value}));
    let rendered = header(&headers, "x-litellm-fastest_response_batch_completion");
    assert_eq!(rendered.unwrap_or_default(), expected);
}

#[test]
fn timeout_is_python_float_text() {
    let headers = build(json!({"timeout": 30.0}));
    assert_eq!(
        header(&headers, "x-litellm-timeout").as_deref(),
        Some("30.0")
    );
}

#[test]
fn a_float_number_uses_python_float_text() {
    let headers = build(json!({"extra": [["x-small", 0.00001], ["x-big", 1e16]]}));
    assert_eq!(header(&headers, "x-small").as_deref(), Some("1e-05"));
    assert_eq!(header(&headers, "x-big").as_deref(), Some("1e+16"));
}

#[test]
fn remaining_limits_use_the_h11_safe_group_name() {
    let headers = build(json!({
        "request_data": {
            "metadata": {
                "model_group": "openai/gpt:4o",
                "litellm-key-remaining-requests-openai/gpt:4o": 5,
                "litellm-key-remaining-tokens-openai/gpt:4o": 100
            }
        },
        "extra": [["x-litellm-key-remaining-requests-openai-gpt-4o", "99"]]
    }));
    assert_eq!(
        header(&headers, "x-litellm-key-remaining-requests-openai-gpt-4o").as_deref(),
        Some("5")
    );
    assert_eq!(
        header(&headers, "x-litellm-key-remaining-tokens-openai-gpt-4o").as_deref(),
        Some("100")
    );
}

#[test]
fn remaining_limits_use_none_when_the_model_group_is_missing() {
    let headers = build(json!({
        "request_data": {"metadata": {"litellm-key-remaining-requests-None": 7}}
    }));
    assert_eq!(
        header(&headers, "x-litellm-key-remaining-requests-None").as_deref(),
        Some("7")
    );
}

#[test]
fn a_falsey_remaining_limit_leaves_its_header_out() {
    let headers = build(json!({
        "request_data": {"metadata": {
            "model_group": "group",
            "litellm-key-remaining-requests-group": 0,
            "litellm-key-remaining-tokens-group": ""
        }}
    }));
    assert_eq!(
        header(&headers, "x-litellm-key-remaining-requests-group"),
        None
    );
    assert_eq!(
        header(&headers, "x-litellm-key-remaining-tokens-group"),
        None
    );
}

#[test]
fn a_non_object_metadata_bucket_adds_no_remaining_headers() {
    let headers = build(json!({"request_data": {"metadata": "text"}}));
    assert_eq!(
        header(&headers, "x-litellm-key-remaining-requests-None"),
        None
    );
    assert_eq!(
        header(&headers, "x-litellm-key-spend").as_deref(),
        Some("0.0")
    );
}

#[test]
fn a_non_scalar_model_group_leaves_the_remaining_headers_out() {
    let headers = build(json!({
        "request_data": {"metadata": {
            "model_group": {"a": 1},
            "litellm-key-remaining-requests-None": 5
        }}
    }));
    assert_eq!(
        header(&headers, "x-litellm-key-remaining-requests-None"),
        None
    );
}

#[test]
fn guardrail_headers_come_from_the_request_data() {
    let headers = build(json!({
        "request_data": {"metadata": {"applied_guardrails": ["gr-1", "gr-2"]}},
        "extra": [["x-litellm-applied-guardrails", "stale"]]
    }));
    assert_eq!(
        header(&headers, "x-litellm-applied-guardrails").as_deref(),
        Some("gr-1,gr-2")
    );
}

#[test]
fn a_bad_guardrail_field_drops_the_headers() {
    let headers = build(json!({
        "request_data": {"metadata": {"applied_guardrails": [1]}}
    }));
    assert_eq!(header(&headers, "x-litellm-applied-guardrails"), None);
    assert_eq!(
        header(&headers, "x-litellm-key-spend").as_deref(),
        Some("0.0")
    );
}

#[test]
fn null_empty_and_none_text_values_are_left_out() {
    let headers = build(json!({
        "model_id": "model-1",
        "cache_key": "",
        "version": "None",
        "response_cost": null,
        "logging": {"cost_breakdown": {"original_cost": null}}
    }));
    assert_eq!(
        header(&headers, "x-litellm-model-id").as_deref(),
        Some("model-1")
    );
    assert_eq!(header(&headers, "x-litellm-cache-key"), None);
    assert_eq!(header(&headers, "x-litellm-version"), None);
    assert_eq!(header(&headers, "x-litellm-response-cost"), None);
    assert_eq!(header(&headers, "x-litellm-response-cost-original"), None);
}

#[test]
fn headers_axum_cannot_carry_are_left_out() {
    let headers = build(json!({
        "extra": [["x-bad\nname", "value"], ["x-bad-value", "a\nb"], ["x-list", [1, 2]]]
    }));
    assert_eq!(
        header(&headers, "x-litellm-key-spend").as_deref(),
        Some("0.0")
    );
    assert!(!headers.keys().any(|name| name.as_str().contains("bad")));
    assert_eq!(header(&headers, "x-list"), None);
}
