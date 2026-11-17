use betterlite_error::error::ErrorClass;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::{
    BUDGET_MESSAGE, SCHEMA_MESSAGE, authentication, bad_request, budget, errors_header, guardrail,
    header, info, json, object, plain, rate_limit, schema, unsupported,
};
use crate::fallback_observations::details::truncate_fallback_error_detail;
use crate::fallback_observations::error_info::{
    FALLBACK_ERRORS_HEADER, FallbackErrorInfo, fallback_error_info,
};
use crate::fallback_observations::headers::{ATTEMPTED_FALLBACKS_HEADER, attach_fallback_headers};
use crate::router_core::headers::Response;

#[rstest]
#[case::rate_limit(
    rate_limit("rate limit reached", Some("messages")),
    info(
        "rate limit reached",
        "throttling_error",
        Some("messages"),
        Some("429")
    )
)]
#[case::bad_request_with_a_body(
    bad_request("invalid request", Some("invalid_request_error"), Some("messages")),
    info(
        "invalid request",
        "invalid_request_error",
        Some("messages"),
        Some("400")
    )
)]
#[case::bad_request_without_a_body(
    bad_request("invalid request", None, None),
    info("invalid request", "None", None, Some("400"))
)]
#[case::authentication(
    authentication("invalid api key"),
    info("invalid api key", "None", None, Some("401"))
)]
#[case::budget(
    budget(Some("custom budget message")),
    info("custom budget message", "BudgetExceededError", None, Some("422"))
)]
#[case::guardrail(
    guardrail("blocked by pii"),
    info("blocked by pii", "GuardrailRaisedException", None, Some("400"))
)]
#[case::unsupported_params(
    unsupported("unsupported parameter"),
    info("unsupported parameter", "UnsupportedParamsError", None, Some("400"))
)]
#[case::json_schema(schema(), info(SCHEMA_MESSAGE, "None", None, Some("500")))]
#[case::unclassified(plain("plain failure"), info("plain failure", "Exception", None, None))]
fn fallback_error_info_reads_each_family(
    #[case] exception: ErrorClass,
    #[case] expected: FallbackErrorInfo,
) {
    assert_eq!(fallback_error_info(&exception), expected);
}

#[rstest]
#[case::absent_message(None)]
#[case::empty_message(Some(""))]
fn budget_without_a_message_reports_the_costs(#[case] message: Option<&str>) {
    assert_eq!(
        fallback_error_info(&budget(message)).message,
        BUDGET_MESSAGE
    );
}

#[rstest]
#[case::one_below(1999)]
#[case::at_the_limit(2000)]
#[case::one_above(2001)]
#[case::two_above(2002)]
fn detail_truncates_only_past_the_litellm_limit(#[case] length: usize) {
    let message = "x".repeat(length);
    let kept: String = message.chars().take(2000).collect();
    let expected = if length <= 2000 {
        kept
    } else {
        let dropped = length.saturating_sub(2000);
        format!("{kept}... [truncated {dropped} characters]")
    };
    assert_eq!(truncate_fallback_error_detail(&message), expected);
}

#[test]
fn detail_counts_code_points_not_bytes() {
    let message = format!("{}é", "a".repeat(2000));
    assert_eq!(
        truncate_fallback_error_detail(&message),
        format!("{}... [truncated 1 characters]", "a".repeat(2000))
    );
}

#[test]
fn detail_cuts_between_code_points() {
    let message = format!("{}éb", "a".repeat(1999));
    assert_eq!(
        truncate_fallback_error_detail(&message),
        format!("{}é... [truncated 1 characters]", "a".repeat(1999))
    );
}

#[test]
fn a_non_object_existing_error_entry_is_dropped() {
    let body = object(
        r#"{"_hidden_params":{"additional_headers":{"x-litellm-fallback-errors":["text", 1, {"message":"kept","type":"Exception","param":null,"code":null}]}}}"#,
    );
    let response = attach_fallback_headers(Response::Object(body), 1, Some(&[]));
    assert_eq!(
        errors_header(&response),
        json(r#"[{"message":"kept","type":"Exception","param":null,"code":null}]"#)
    );
}

#[test]
fn an_invalid_json_string_contributes_nothing() {
    let body = object(
        r#"{"_hidden_params":{"additional_headers":{"x-litellm-fallback-errors":"[not json"}}}"#,
    );
    let errors = [info("only", "Exception", None, None)];
    let response = attach_fallback_headers(Response::Object(body), 1, Some(&errors));
    assert_eq!(
        errors_header(&response),
        json(r#"[{"message":"only","type":"Exception","param":null,"code":null}]"#)
    );
}

#[test]
fn a_json_string_that_parses_to_a_non_list_contributes_nothing() {
    let body =
        object(r#"{"_hidden_params":{"additional_headers":{"x-litellm-fallback-errors":"{}"}}}"#);
    let errors = [info("only", "Exception", None, None)];
    let response = attach_fallback_headers(Response::Object(body), 1, Some(&errors));
    assert_eq!(
        errors_header(&response),
        json(r#"[{"message":"only","type":"Exception","param":null,"code":null}]"#)
    );
}

#[test]
fn a_header_that_is_neither_list_nor_string_contributes_nothing() {
    let body =
        object(r#"{"_hidden_params":{"additional_headers":{"x-litellm-fallback-errors":5}}}"#);
    let errors = [info("only", "Exception", None, None)];
    let response = attach_fallback_headers(Response::Object(body), 1, Some(&errors));
    assert_eq!(
        errors_header(&response),
        json(r#"[{"message":"only","type":"Exception","param":null,"code":null}]"#)
    );
}

#[test]
fn an_empty_error_slice_is_written_as_an_empty_array() {
    let response = attach_fallback_headers(Response::Object(JsonObject::new()), 0, Some(&[]));
    assert_eq!(
        header(&response, ATTEMPTED_FALLBACKS_HEADER),
        Some(json("0"))
    );
    assert_eq!(
        header(&response, FALLBACK_ERRORS_HEADER),
        Some(json(r#""[]""#))
    );
}

#[test]
fn absent_errors_leave_the_existing_header_alone() {
    let existing = r#"[{"message":"kept","type":"Exception","param":null,"code":null}]"#;
    let body = object(&format!(
        r#"{{"_hidden_params":{{"additional_headers":{{"x-litellm-fallback-errors":{existing}}}}}}}"#
    ));
    let response = attach_fallback_headers(Response::Object(body), 1, None);
    assert_eq!(
        header(&response, FALLBACK_ERRORS_HEADER),
        Some(json(existing))
    );
}
