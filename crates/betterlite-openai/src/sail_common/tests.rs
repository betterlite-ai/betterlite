#![cfg(test)]

use betterlite_error::error::ErrorClass;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde_json::{Number, Value, json};

use super::error::SailError;
use super::{
    CompletionWindow, SailSettings, chat_request_for_sail, completion_window_for_service_tier,
    describe, extra_body_for_sail, metadata_without_caller_window,
    responses_params_with_completion_window,
};

const MODEL: &str = "sail/llama-3-70b";
const HINT: &str = "To drop it, set `litellm.drop_params=True` or for proxy: `litellm_settings: drop_params: true`";

fn object(value: Value) -> Object {
    serde_json::from_value(value).unwrap()
}

fn value(object: &Object) -> Value {
    serde_json::to_value(object).unwrap()
}

fn text(source: &str) -> Json {
    Json::Text(source.to_owned())
}

fn settings(global_drop_params: bool) -> SailSettings {
    SailSettings { global_drop_params }
}

fn window_of(
    service_tier: Option<&Json>,
    drop_params: bool,
    settings: &SailSettings,
) -> Option<&'static str> {
    completion_window_for_service_tier(service_tier, MODEL, drop_params, settings)
        .unwrap()
        .map(CompletionWindow::as_str)
}

#[rstest]
#[case("auto", None)]
#[case("default", Some("asap"))]
#[case("priority", Some("asap"))]
#[case("flex", Some("flex"))]
#[case("balanced", Some("balanced"))]
#[case("FLEX", Some("flex"))]
#[case("Balanced", Some("balanced"))]
#[case("DEFAULT", Some("asap"))]
#[case("Priority", Some("asap"))]
#[case("AUTO", None)]
fn known_service_tiers_translate_to_windows(#[case] tier: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        window_of(Some(&text(tier)), false, &settings(false)),
        expected
    );
}

#[test]
fn a_null_or_absent_tier_gives_no_window() {
    assert_eq!(window_of(None, false, &settings(false)), None);
    assert_eq!(window_of(Some(&Json::Null), false, &settings(false)), None);
}

#[rstest]
#[case(json!("flash"))]
#[case(json!(5))]
#[case(json!(true))]
#[case(json!([1, 2]))]
#[case(json!({"tier": "flex"}))]
fn an_unknown_tier_errors_without_dropping(#[case] given: Value) {
    let tier: Json = serde_json::from_value(given).unwrap();
    let error = completion_window_for_service_tier(Some(&tier), MODEL, false, &settings(false))
        .unwrap_err();
    assert_eq!(error.status_code(), 400);
}

#[test]
fn the_unknown_tier_error_names_the_tier_and_the_supported_values() {
    let error =
        completion_window_for_service_tier(Some(&text("flash")), MODEL, false, &settings(false))
            .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "sail does not support service_tier=\"flash\". Supported values: auto, default, priority, flex, balanced. {HINT}"
        )
    );
}

#[test]
fn an_unknown_tier_is_dropped_by_the_argument_or_the_global_setting() {
    assert_eq!(
        window_of(Some(&text("flash")), true, &settings(false)),
        None
    );
    assert_eq!(
        window_of(Some(&text("flash")), false, &settings(true)),
        None
    );
    assert_eq!(window_of(Some(&text("flash")), true, &settings(true)), None);
    let number = Json::Number(Number::from(15_u64));
    assert_eq!(window_of(Some(&number), true, &settings(false)), None);
    assert_eq!(
        window_of(Some(&Json::Bool(true)), false, &settings(true)),
        None
    );
}

#[test]
fn a_dropped_tier_still_keeps_a_known_one() {
    assert_eq!(
        window_of(Some(&text("flex")), true, &settings(true)),
        Some("flex")
    );
    assert_eq!(window_of(Some(&text("auto")), true, &settings(true)), None);
}

#[test]
fn the_error_class_carries_the_sail_provider_and_the_model() {
    let error =
        completion_window_for_service_tier(Some(&text("flash")), MODEL, false, &settings(false))
            .unwrap_err();
    let ErrorClass::UnsupportedParams(context) = error.class() else {
        panic!("the service tier error is an unsupported params error");
    };
    assert_eq!(context.provider.as_deref(), Some("sail"));
    assert_eq!(context.model.as_deref(), Some(MODEL));
    assert_eq!(context.message, error.to_string());
}

#[test]
fn metadata_without_the_caller_window_keeps_the_other_keys() {
    let metadata = object(json!({"user_id": "u-1", "completion_window": "flex"}));
    let error =
        metadata_without_caller_window(Some(&metadata), "metadata", MODEL, false, &settings(false))
            .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "sail does not accept metadata.completion_window. Send service_tier instead. {HINT}"
        )
    );
    let cleaned =
        metadata_without_caller_window(Some(&metadata), "metadata", MODEL, true, &settings(false))
            .unwrap();
    assert_eq!(value(&cleaned), json!({"user_id": "u-1"}));
    let global =
        metadata_without_caller_window(Some(&metadata), "metadata", MODEL, false, &settings(true))
            .unwrap();
    assert_eq!(value(&global), json!({"user_id": "u-1"}));
}

#[rstest]
#[case(None)]
#[case(Some(Json::Null))]
#[case(Some(Json::Text(String::from("x"))))]
#[case(Some(Json::List(Vec::new())))]
#[case(Some(Json::Number(Number::from(15_u64))))]
fn metadata_without_the_caller_window_is_empty_for_non_mappings(#[case] metadata: Option<Json>) {
    let cleaned = metadata_without_caller_window(
        metadata.as_ref(),
        "metadata",
        MODEL,
        false,
        &settings(false),
    )
    .unwrap();
    assert!(cleaned.is_empty());
}

#[test]
fn the_extra_body_field_name_reaches_the_error() {
    let metadata = object(json!({"completion_window": "flex"}));
    let error = metadata_without_caller_window(
        Some(&metadata),
        "extra_body.metadata",
        MODEL,
        false,
        &settings(false),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "sail does not accept extra_body.metadata.completion_window. Send service_tier instead. {HINT}"
        )
    );
}

#[test]
fn a_service_tier_inside_extra_body_errors_without_dropping() {
    let extra_body = object(json!({"service_tier": "flex", "user": "u-1"}));
    let error = extra_body_for_sail(&extra_body, None, MODEL, false, &settings(false)).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "sail does not accept service_tier inside extra_body. Send service_tier instead. {HINT}"
        )
    );
    let by_argument =
        extra_body_for_sail(&extra_body, None, MODEL, true, &settings(false)).unwrap();
    assert_eq!(value(&by_argument), json!({"user": "u-1"}));
    let by_global = extra_body_for_sail(&extra_body, None, MODEL, false, &settings(true)).unwrap();
    assert_eq!(value(&by_global), json!({"user": "u-1"}));
}

#[test]
fn extra_body_merges_the_request_metadata_over_the_caller_metadata() {
    let extra_body = object(json!({"metadata": {"user_id": "u-1", "team": "a"}, "user": "u-2"}));
    let request_metadata = object(json!({"team": "b", "completion_window": "flex"}));
    let merged = extra_body_for_sail(
        &extra_body,
        Some(&Json::Object(request_metadata)),
        MODEL,
        false,
        &settings(false),
    )
    .unwrap();
    assert_eq!(
        value(&merged),
        json!({"user": "u-2", "metadata": {"user_id": "u-1", "team": "b", "completion_window": "flex"}})
    );
}

#[test]
fn extra_body_ignores_a_non_mapping_request_metadata() {
    let extra_body = object(json!({"metadata": {"user_id": "u-1"}}));
    let merged = extra_body_for_sail(
        &extra_body,
        Some(&text("raw")),
        MODEL,
        false,
        &settings(false),
    )
    .unwrap();
    assert_eq!(value(&merged), json!({"metadata": {"user_id": "u-1"}}));
}

#[rstest]
#[case(json!("raw"))]
#[case(json!(null))]
#[case(json!(5))]
#[case(json!([1]))]
fn extra_body_keeps_a_non_mapping_metadata_when_nothing_merges(#[case] metadata: Value) {
    let extra_body = object(json!({"metadata": metadata.clone(), "user": "u"}));
    let kept = extra_body_for_sail(&extra_body, None, MODEL, false, &settings(false)).unwrap();
    assert_eq!(value(&kept), json!({"user": "u", "metadata": metadata}));
}

#[test]
fn extra_body_drops_an_empty_or_cleaned_metadata() {
    let empty = object(json!({"metadata": {}, "user": "u"}));
    let kept_empty = extra_body_for_sail(&empty, None, MODEL, false, &settings(false)).unwrap();
    assert_eq!(value(&kept_empty), json!({"user": "u"}));
    let caller = object(json!({"metadata": {"completion_window": "flex"}, "user": "u"}));
    let kept_cleaned = extra_body_for_sail(&caller, None, MODEL, true, &settings(false)).unwrap();
    assert_eq!(value(&kept_cleaned), json!({"user": "u"}));
}

#[test]
fn extra_body_without_metadata_sends_only_the_rest() {
    let extra_body = object(json!({"user": "u", "service_tier": "flex"}));
    let kept = extra_body_for_sail(&extra_body, None, MODEL, true, &settings(false)).unwrap();
    assert_eq!(value(&kept), json!({"user": "u"}));
}

#[test]
fn extra_body_metadata_carrying_only_the_caller_window_is_rejected() {
    let extra_body = object(json!({"metadata": {"completion_window": "flex"}}));
    let error = extra_body_for_sail(&extra_body, None, MODEL, false, &settings(false)).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "sail does not accept extra_body.metadata.completion_window. Send service_tier instead. {HINT}"
        )
    );
}

#[test]
fn a_chat_request_moves_the_tier_into_the_metadata_window() {
    let request = object(json!({
        "model": "llama-3-70b",
        "messages": [{"role": "user", "content": "hi"}],
        "service_tier": "flex",
    }));
    let rewritten = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({
            "model": "llama-3-70b",
            "messages": [{"role": "user", "content": "hi"}],
            "metadata": {"completion_window": "flex"},
        })
    );
}

#[test]
fn a_chat_request_keeps_the_caller_metadata_and_adds_the_window() {
    let request = object(json!({"service_tier": "default", "metadata": {"user_id": "u-1"}}));
    let rewritten = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"metadata": {"user_id": "u-1", "completion_window": "asap"}})
    );
}

#[test]
fn a_chat_request_without_a_tier_keeps_the_caller_metadata() {
    let request = object(json!({"metadata": {"user_id": "u-1"}}));
    let rewritten = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap();
    assert_eq!(value(&rewritten), json!({"metadata": {"user_id": "u-1"}}));
}

#[test]
fn a_chat_request_rejects_a_caller_window_in_metadata() {
    let request = object(json!({"metadata": {"completion_window": "asap"}}));
    let error = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "sail does not accept metadata.completion_window. Send service_tier instead. {HINT}"
        )
    );
}

#[test]
fn a_chat_request_cleans_a_caller_window_when_dropping() {
    let request = object(json!({
        "service_tier": "flex",
        "metadata": {"completion_window": "asap", "user_id": "u-1"},
    }));
    let rewritten = chat_request_for_sail(&request, MODEL, true, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"metadata": {"user_id": "u-1", "completion_window": "flex"}})
    );
}

#[rstest]
#[case(json!("raw"))]
#[case(json!([1]))]
#[case(json!(null))]
#[case(json!(5))]
fn a_chat_request_drops_a_non_mapping_metadata(#[case] metadata: Value) {
    let request = object(json!({"metadata": metadata, "temperature": 0.2}));
    let rewritten = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap();
    assert_eq!(value(&rewritten), json!({"temperature": 0.2}));
}

#[test]
fn a_chat_request_rebuilds_a_non_mapping_metadata_from_the_tier() {
    let request = object(json!({"metadata": "raw", "service_tier": "flex"}));
    let rewritten = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"metadata": {"completion_window": "flex"}})
    );
}

#[test]
fn a_chat_request_merges_a_mapping_extra_body() {
    let request = object(json!({
        "service_tier": "balanced",
        "extra_body": {"user": "u-1", "metadata": {"trace": "t-1"}},
        "temperature": 0.2,
    }));
    let rewritten = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({
            "temperature": 0.2,
            "metadata": {"completion_window": "balanced"},
            "extra_body": {"user": "u-1", "metadata": {"trace": "t-1", "completion_window": "balanced"}},
        })
    );
}

#[rstest]
#[case(json!("raw"))]
#[case(json!([1]))]
#[case(json!(null))]
#[case(json!(7))]
fn a_chat_request_drops_a_non_mapping_extra_body(#[case] extra_body: Value) {
    let request = object(json!({"extra_body": extra_body, "temperature": 0.2}));
    let rewritten = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap();
    assert_eq!(value(&rewritten), json!({"temperature": 0.2}));
}

#[test]
fn a_chat_request_keeps_an_empty_extra_body() {
    let request = object(json!({"extra_body": {}, "temperature": 0.2}));
    let rewritten = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"temperature": 0.2, "extra_body": {}})
    );
}

#[test]
fn a_chat_request_rejects_a_tier_inside_extra_body() {
    let request = object(json!({"extra_body": {"service_tier": "flex"}}));
    let error = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap_err();
    assert_eq!(error.status_code(), 400);
    let rewritten = chat_request_for_sail(&request, MODEL, true, &settings(false)).unwrap();
    assert_eq!(value(&rewritten), json!({"extra_body": {}}));
}

#[test]
fn a_chat_request_keeps_the_unknown_keys() {
    let request = object(json!({
        "model": "m",
        "stream": true,
        "max_tokens": 5,
        "service_tier": "auto",
    }));
    let rewritten = chat_request_for_sail(&request, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"model": "m", "stream": true, "max_tokens": 5})
    );
}

#[test]
fn responses_rewrite_the_tier_into_the_billed_one() {
    let params = object(json!({"service_tier": "default", "user": "u-1"}));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"user": "u-1", "metadata": {"completion_window": "asap"}})
    );
}

#[test]
fn responses_map_the_window_back_to_its_billed_tier() {
    let flex_params = object(json!({"service_tier": "flex"}));
    let flex_result =
        responses_params_with_completion_window(&flex_params, MODEL, false, &settings(false))
            .unwrap();
    assert_eq!(
        value(&flex_result),
        json!({"metadata": {"completion_window": "flex"}, "service_tier": "flex"})
    );
    let balanced_params = object(json!({"service_tier": "balanced"}));
    let balanced_result =
        responses_params_with_completion_window(&balanced_params, MODEL, false, &settings(false))
            .unwrap();
    assert_eq!(
        value(&balanced_result),
        json!({"metadata": {"completion_window": "balanced"}, "service_tier": "balanced"})
    );
}

#[test]
fn responses_leave_asap_without_a_service_tier() {
    let params = object(json!({"service_tier": "priority", "metadata": {"user_id": "u-1"}}));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"metadata": {"user_id": "u-1", "completion_window": "asap"}})
    );
}

#[test]
fn responses_auto_removes_the_tier_and_keeps_a_null_metadata_out() {
    let params = object(json!({"service_tier": "auto", "metadata": null}));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(value(&rewritten), json!({}));
}

#[test]
fn responses_keep_an_empty_mapping_metadata() {
    let params = object(json!({"service_tier": "auto", "metadata": {}}));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(value(&rewritten), json!({"metadata": {}}));
}

#[test]
fn responses_keep_a_non_mapping_metadata_as_an_empty_object() {
    let params = object(json!({"metadata": "raw", "user": "u-1"}));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(value(&rewritten), json!({"user": "u-1", "metadata": {}}));
}

#[test]
fn responses_let_the_caller_window_win_over_the_tier() {
    let params = object(json!({
        "service_tier": "default",
        "metadata": {"completion_window": "asap", "user_id": "u-1"},
    }));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"metadata": {"user_id": "u-1", "completion_window": "asap"}})
    );
}

#[test]
fn responses_lowercase_a_caller_standard_window() {
    let params = object(json!({"metadata": {"completion_window": "Standard"}}));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"metadata": {"completion_window": "standard"}, "service_tier": "balanced"})
    );
}

#[test]
fn responses_reject_two_windows_with_different_billed_tiers() {
    let conflict = object(json!({
        "service_tier": "default",
        "metadata": {"completion_window": "flex"},
    }));
    let error = responses_params_with_completion_window(&conflict, MODEL, false, &settings(false))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "sail got service_tier=\"default\" and metadata.completion_window=\"flex\", which select different completion windows. Send one of them. {HINT}"
        )
    );
    let swapped =
        object(json!({"service_tier": "flex", "metadata": {"completion_window": "standard"}}));
    let swapped_error =
        responses_params_with_completion_window(&swapped, MODEL, false, &settings(false))
            .unwrap_err();
    assert_eq!(swapped_error.status_code(), 400);
}

#[test]
fn responses_accept_two_windows_that_bill_the_same() {
    let params = object(json!({
        "service_tier": "balanced",
        "metadata": {"completion_window": "standard"},
    }));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"metadata": {"completion_window": "standard"}, "service_tier": "balanced"})
    );
}

#[test]
fn responses_reject_an_unknown_caller_window_without_dropping() {
    let unknown = object(json!({"metadata": {"completion_window": "slow"}}));
    let unknown_error =
        responses_params_with_completion_window(&unknown, MODEL, false, &settings(false))
            .unwrap_err();
    assert_eq!(
        unknown_error.to_string(),
        format!(
            "sail does not support metadata.completion_window=\"slow\". Supported values: asap, balanced, standard, flex. {HINT}"
        )
    );
    let non_string = object(json!({"metadata": {"completion_window": 5}}));
    let non_string_error =
        responses_params_with_completion_window(&non_string, MODEL, false, &settings(false))
            .unwrap_err();
    assert_eq!(non_string_error.status_code(), 400);
}

#[test]
fn responses_drop_an_unknown_caller_window_through_the_argument_or_the_global() {
    let params = object(json!({"metadata": {"completion_window": "slow", "user_id": "u-1"}}));
    let by_argument =
        responses_params_with_completion_window(&params, MODEL, true, &settings(false)).unwrap();
    assert_eq!(value(&by_argument), json!({"metadata": {"user_id": "u-1"}}));
    let by_global =
        responses_params_with_completion_window(&params, MODEL, false, &settings(true)).unwrap();
    assert_eq!(value(&by_global), json!({"metadata": {"user_id": "u-1"}}));
}

#[test]
fn responses_without_a_tier_rewrite_a_caller_window_alone() {
    let params = object(json!({"metadata": {"completion_window": "Flex", "user_id": "u-1"}}));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"metadata": {"user_id": "u-1", "completion_window": "flex"}, "service_tier": "flex"})
    );
}

#[test]
fn responses_ignore_a_tier_whose_window_is_auto() {
    let params = object(json!({"service_tier": "auto"}));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(value(&rewritten), json!({}));
}

#[test]
fn responses_without_any_window_keep_the_params() {
    let params = object(json!({"user": "u-1", "temperature": 0.2}));
    let rewritten =
        responses_params_with_completion_window(&params, MODEL, false, &settings(false)).unwrap();
    assert_eq!(
        value(&rewritten),
        json!({"user": "u-1", "temperature": 0.2})
    );
}

#[test]
fn every_unsupported_error_ends_with_the_drop_params_hint() {
    let tier =
        completion_window_for_service_tier(Some(&text("flash")), MODEL, false, &settings(false))
            .unwrap_err();
    let metadata = metadata_without_caller_window(
        Some(&object(json!({"completion_window": "flex"}))),
        "metadata",
        MODEL,
        false,
        &settings(false),
    )
    .unwrap_err();
    let extra = extra_body_for_sail(
        &object(json!({"service_tier": "flex"})),
        None,
        MODEL,
        false,
        &settings(false),
    )
    .unwrap_err();
    let window = responses_params_with_completion_window(
        &object(json!({"metadata": {"completion_window": "slow"}})),
        MODEL,
        false,
        &settings(false),
    )
    .unwrap_err();
    let conflict = responses_params_with_completion_window(
        &object(json!({"service_tier": "default", "metadata": {"completion_window": "flex"}})),
        MODEL,
        false,
        &settings(false),
    )
    .unwrap_err();
    assert!(tier.to_string().ends_with(HINT), "{tier}");
    assert_eq!(tier.status_code(), 400);
    assert!(metadata.to_string().ends_with(HINT), "{metadata}");
    assert_eq!(metadata.status_code(), 400);
    assert!(extra.to_string().ends_with(HINT), "{extra}");
    assert_eq!(extra.status_code(), 400);
    assert!(window.to_string().ends_with(HINT), "{window}");
    assert_eq!(window.status_code(), 400);
    assert!(conflict.to_string().ends_with(HINT), "{conflict}");
    assert_eq!(conflict.status_code(), 400);
}

#[test]
fn a_non_string_tier_is_described_by_its_json_text() {
    let number = Json::Number(Number::from(15_u64));
    let error = completion_window_for_service_tier(Some(&number), MODEL, false, &settings(false))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("sail does not support service_tier=15."),
        "{error}"
    );
    let SailError::UnsupportedServiceTier { model, .. } = error else {
        panic!("the service tier error keeps the model");
    };
    assert_eq!(model, MODEL);
}

#[test]
fn a_description_is_debug_text_for_strings_and_json_text_for_other_values() {
    assert_eq!(describe(Some(&text("a\"b"))), "\"a\\\"b\"");
    assert_eq!(describe(Some(&Json::Number(Number::from(15_u64)))), "15");
    assert_eq!(describe(Some(&Json::Bool(true))), "true");
    assert_eq!(describe(Some(&Json::List(Vec::new()))), "[]");
    assert_eq!(describe(None), "null");
}
