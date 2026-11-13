#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::response;
use crate::model_armor::decisions::{
    append_armor_response, filter_result_items, get_sanitized_content, has_deidentify_match,
    should_block_content,
};

fn filter_payload(filter_key: &str, state: &str) -> GuardrailValue {
    let source = format!(
        r#"{{"sanitizationResult":{{"filterResults":{{"filter":{{"{filter_key}":{{"matchState":"{state}"}}}}}}}}}}"#
    );
    response(&source)
}

fn sdp_payload(result_key: &str, state: &str) -> GuardrailValue {
    let source = format!(
        r#"{{"sanitizationResult":{{"filterResults":{{"sdp":{{"sdpFilterResult":{{"{result_key}":{{"matchState":"{state}"}}}}}}}}}}}}"#
    );
    response(&source)
}

#[rstest]
#[case::rai("raiFilterResult")]
#[case::pi_and_jailbreak("piAndJailbreakFilterResult")]
#[case::malicious_uri("maliciousUriFilterResult")]
#[case::csam("csamFilterFilterResult")]
#[case::virus("virusScanFilterResult")]
fn should_block_content_blocks_on_each_filter_match(#[case] filter_key: &str) {
    let payload = filter_payload(filter_key, "MATCH_FOUND");
    assert!(should_block_content(&payload, false));
    assert!(should_block_content(&payload, true));
}

#[test]
fn should_block_content_allows_a_clean_response() {
    let no_match = filter_payload("raiFilterResult", "NO_MATCH_FOUND");
    assert!(!should_block_content(&no_match, false));
    let lower_case = filter_payload("raiFilterResult", "match_found");
    assert!(!should_block_content(&lower_case, false));
    assert!(!should_block_content(&response("{}"), false));
}

#[test]
fn should_block_content_blocks_on_an_inspect_match_even_when_sanitizing() {
    let payload = sdp_payload("inspectResult", "MATCH_FOUND");
    assert!(should_block_content(&payload, false));
    assert!(should_block_content(&payload, true));
}

#[test]
fn should_block_content_blocks_a_deidentify_match_only_without_sanitization() {
    let payload = sdp_payload("deidentifyResult", "MATCH_FOUND");
    assert!(should_block_content(&payload, false));
    assert!(!should_block_content(&payload, true));
}

#[test]
fn should_block_content_allows_a_sdp_result_that_matched_nothing() {
    let payload = sdp_payload("deidentifyResult", "NO_MATCH_FOUND");
    assert!(!should_block_content(&payload, false));
}

#[test]
fn should_block_content_reads_a_list_of_filter_results() {
    let blocking = response(
        r#"{"sanitizationResult":{"filterResults":[{"raiFilterResult":{"matchState":"MATCH_FOUND"}}]}}"#,
    );
    assert!(should_block_content(&blocking, false));
    let clean = response(
        r#"{"sanitizationResult":{"filterResults":[{"raiFilterResult":{"matchState":"NO_MATCH_FOUND"}}]}}"#,
    );
    assert!(!should_block_content(&clean, false));
}

#[test]
fn filter_result_items_flattens_dicts_and_lists() {
    let named = response(
        r#"{"sanitizationResult":{"filterResults":{"a":{"raiFilterResult":{}},"b":{"virusScanFilterResult":{}}}}}"#,
    );
    assert_eq!(filter_result_items(&named).len(), 2);
    let listed = response(r#"{"sanitizationResult":{"filterResults":[{"a":1}]}}"#);
    assert_eq!(filter_result_items(&listed).len(), 1);
    let empty = response(r#"{"sanitizationResult":{"filterResults":{}}}"#);
    assert!(filter_result_items(&empty).is_empty());
    let missing = response(r#"{"sanitizationResult":{}}"#);
    assert!(filter_result_items(&missing).is_empty());
    let not_a_result = response(r#"{"text":"hello"}"#);
    assert!(filter_result_items(&not_a_result).is_empty());
}

#[test]
fn get_sanitized_content_returns_the_deidentify_text() {
    let payload = response(
        r#"{"sanitizationResult":{"filterResults":{"sdp":{"sdpFilterResult":{"deidentifyResult":{"matchState":"MATCH_FOUND","data":{"text":"masked@example.test"}}}}}}}"#,
    );
    assert_eq!(
        get_sanitized_content(&payload),
        Some("masked@example.test".to_owned())
    );
}

#[test]
fn get_sanitized_content_skips_an_empty_deidentify_text() {
    let payload = response(
        r#"{"sanitizationResult":{"filterResults":{"sdp":{"sdpFilterResult":{"deidentifyResult":{"matchState":"MATCH_FOUND","data":{"text":""}}}}}},"sanitizedText":"root"}"#,
    );
    assert_eq!(get_sanitized_content(&payload), Some("root".to_owned()));
}

#[test]
fn get_sanitized_content_ignores_a_deidentify_result_that_did_not_match() {
    let payload = response(
        r#"{"sanitizationResult":{"filterResults":{"sdp":{"sdpFilterResult":{"deidentifyResult":{"matchState":"NO_MATCH_FOUND","data":{"text":"leaked"}}}}}}}"#,
    );
    assert_eq!(get_sanitized_content(&payload), None);
}

#[test]
fn get_sanitized_content_returns_none_for_an_inspect_match_without_output() {
    let payload = response(
        r#"{"sanitizationResult":{"filterResults":{"sdp":{"sdpFilterResult":{"inspectResult":{"matchState":"MATCH_FOUND"}}}}},"sanitizedText":"root"}"#,
    );
    assert_eq!(get_sanitized_content(&payload), None);
}

#[test]
fn get_sanitized_content_falls_back_to_the_root_fields() {
    let sanitized = response(r#"{"sanitizedText":"clean"}"#);
    assert_eq!(get_sanitized_content(&sanitized), Some("clean".to_owned()));
    let text = response(r#"{"text":"plain"}"#);
    assert_eq!(get_sanitized_content(&text), Some("plain".to_owned()));
    let empty_sanitized = response(r#"{"sanitizedText":"","text":"plain"}"#);
    assert_eq!(
        get_sanitized_content(&empty_sanitized),
        Some("plain".to_owned())
    );
    let empty = response(r#"{"sanitizedText":""}"#);
    assert_eq!(get_sanitized_content(&empty), None);
    let empty_text = response(r#"{"text":""}"#);
    assert_eq!(get_sanitized_content(&empty_text), None);
    assert_eq!(get_sanitized_content(&response("{}")), None);
}

#[test]
fn has_deidentify_match_reads_sdp_deidentify_results() {
    assert!(has_deidentify_match(&sdp_payload(
        "deidentifyResult",
        "MATCH_FOUND"
    )));
    assert!(!has_deidentify_match(&sdp_payload(
        "inspectResult",
        "MATCH_FOUND"
    )));
    assert!(!has_deidentify_match(&sdp_payload(
        "deidentifyResult",
        "NO_MATCH_FOUND"
    )));
    assert!(!has_deidentify_match(&response("{}")));
    let listed = response(
        r#"{"sanitizationResult":{"filterResults":[{"sdpFilterResult":{"deidentifyResult":{"matchState":"MATCH_FOUND"}}}]}}"#,
    );
    assert!(has_deidentify_match(&listed));
}

#[test]
fn append_armor_response_returns_the_first_response_on_its_own() {
    let first = response(r#"{"sanitizationResult":{"filterResults":{}}}"#);
    assert_eq!(append_armor_response(None, first.clone()), first);
    assert_eq!(
        append_armor_response(Some(GuardrailValue::Null), first.clone()),
        first
    );
}

#[test]
fn append_armor_response_builds_a_list_from_a_single_response() {
    let first = response(r#"{"scan":"file"}"#);
    let second = response(r#"{"scan":"text"}"#);
    assert_eq!(
        append_armor_response(Some(first), second),
        response(r#"[{"scan":"file"},{"scan":"text"}]"#)
    );
}

#[test]
fn append_armor_response_appends_to_an_existing_list() {
    let existing = response(r#"[{"scan":"file"}]"#);
    let second = response(r#"{"scan":"text"}"#);
    assert_eq!(
        append_armor_response(Some(existing), second),
        response(r#"[{"scan":"file"},{"scan":"text"}]"#)
    );
}

#[test]
fn append_armor_response_wraps_any_other_value() {
    let second = response(r#"{"scan":"text"}"#);
    assert_eq!(
        append_armor_response(Some(GuardrailValue::Text("old".to_owned())), second),
        response(r#"["old",{"scan":"text"}]"#)
    );
}
