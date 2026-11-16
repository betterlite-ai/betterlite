use betterlite_config::sections::guardrail_params::records::LakeraCategoryThresholds;
use reqwest::StatusCode;

use crate::lakera::rejection::{LakeraRejection, LakeraRejectionDetail};
use crate::lakera::response::LakeraModerationResponse;
use crate::lakera::{LegacyLakeraModeration, LegacyLakeraModerationParams};

fn moderation(thresholds: Option<LakeraCategoryThresholds>) -> LegacyLakeraModeration {
    LegacyLakeraModeration::new(
        LegacyLakeraModerationParams {
            moderation_check: None,
            category_thresholds: thresholds,
            api_base: None,
            api_key: None,
        },
        |_key| String::new(),
        reqwest::Client::default,
    )
}

fn thresholds(prompt_injection: Option<f64>, jailbreak: Option<f64>) -> LakeraCategoryThresholds {
    LakeraCategoryThresholds {
        prompt_injection,
        jailbreak,
    }
}

fn parse(body: &str) -> LakeraModerationResponse {
    serde_json::from_str(body).expect("the Lakera response parses")
}

// the check of a configured guardrail on a Lakera response body
fn verdict(configured: Option<LakeraCategoryThresholds>, body: &str) -> Option<LakeraRejection> {
    moderation(configured).check_response_flagged(&parse(body))
}

fn rejection(rejected: Option<LakeraRejection>) -> LakeraRejectionDetail {
    let rejection = rejected.expect("the response is rejected");
    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
    rejection.detail
}

fn assert_echoed(detail: &LakeraRejectionDetail, expected: &str) {
    let written = serde_json::to_string(&detail.lakera_ai_response).expect("the response is JSON");
    assert_eq!(written, expected);
}

#[test]
fn lakera_moderation_blocks_at_the_jailbreak_threshold() {
    let body = r#"{"results":[{"category_scores":{"jailbreak":0.8}}]}"#;
    let rejected = rejection(verdict(Some(thresholds(None, Some(0.8_f64))), body));
    assert_eq!(rejected.error, "Violated jailbreak threshold");
    assert_echoed(&rejected, body);
}

#[test]
fn lakera_moderation_passes_below_the_jailbreak_threshold() {
    let body = r#"{"results":[{"category_scores":{"jailbreak":0.79}}]}"#;
    assert!(verdict(Some(thresholds(None, Some(0.8_f64))), body).is_none());
}

#[test]
fn lakera_moderation_blocks_at_the_prompt_injection_threshold() {
    let body = r#"{"results":[{"category_scores":{"prompt_injection":0.5}}]}"#;
    let rejected = rejection(verdict(Some(thresholds(Some(0.5_f64), None)), body));
    assert_eq!(rejected.error, "Violated prompt_injection threshold");
}

#[test]
fn lakera_moderation_passes_below_the_prompt_injection_threshold() {
    let body = r#"{"results":[{"category_scores":{"prompt_injection":0.49}}]}"#;
    assert!(verdict(Some(thresholds(Some(0.5_f64), None)), body).is_none());
}

#[test]
fn lakera_moderation_reports_a_jailbreak_before_a_prompt_injection() {
    let body = r#"{"results":[{"category_scores":{"jailbreak":0.9,"prompt_injection":0.9}}]}"#;
    let rejected = rejection(verdict(
        Some(thresholds(Some(0.5_f64), Some(0.5_f64))),
        body,
    ));
    assert_eq!(rejected.error, "Violated jailbreak threshold");
}

#[test]
fn lakera_moderation_passes_when_the_breaching_score_has_no_threshold() {
    let body = r#"{"results":[{"category_scores":{"jailbreak":0.99,"prompt_injection":0.1}}]}"#;
    assert!(verdict(Some(thresholds(Some(0.5_f64), None)), body).is_none());
}

#[test]
fn lakera_moderation_passes_when_the_threshold_has_no_score() {
    let body = r#"{"results":[{"category_scores":{"prompt_injection":0.1}}]}"#;
    assert!(verdict(Some(thresholds(Some(0.5_f64), Some(0.5_f64))), body).is_none());
}

#[test]
fn lakera_moderation_ignores_flagged_when_thresholds_have_no_scores() {
    let body = r#"{"results":[{"flagged":true}]}"#;
    assert!(verdict(Some(thresholds(Some(0.5_f64), Some(0.5_f64))), body).is_none());
}

#[test]
fn lakera_moderation_ignores_an_empty_score_object() {
    let body = r#"{"results":[{"flagged":true,"category_scores":{}}]}"#;
    assert!(verdict(Some(thresholds(Some(0.5_f64), Some(0.5_f64))), body).is_none());
}

#[test]
fn lakera_moderation_blocks_a_flagged_response_without_thresholds() {
    let body = r#"{"results":[{"flagged":true,"category_scores":{"jailbreak":0.1}}]}"#;
    let rejected = rejection(verdict(None, body));
    assert_eq!(rejected.error, "Violated content safety policy");
    assert_echoed(&rejected, body);
}

#[test]
fn lakera_moderation_passes_an_unflagged_response_without_thresholds() {
    assert!(verdict(None, r#"{"results":[{"flagged":false}]}"#).is_none());
}

#[test]
fn lakera_moderation_passes_a_response_without_a_flagged_key() {
    let body = r#"{"results":[{"category_scores":{"jailbreak":0.9}}]}"#;
    assert!(verdict(None, body).is_none());
}

#[test]
fn lakera_moderation_passes_a_null_flagged_value() {
    assert!(verdict(None, r#"{"results":[{"flagged":null}]}"#).is_none());
}

#[test]
fn lakera_moderation_passes_a_null_category_scores_map() {
    let body = r#"{"results":[{"flagged":true,"category_scores":null}]}"#;
    assert!(verdict(Some(thresholds(Some(0.5_f64), Some(0.5_f64))), body).is_none());
}

#[test]
fn lakera_moderation_keeps_a_null_score_in_the_detail() {
    let body = r#"{"results":[{"category_scores":{"jailbreak":null,"prompt_injection":0.9}}]}"#;
    let rejected = rejection(verdict(Some(thresholds(Some(0.5_f64), None)), body));
    assert_echoed(
        &rejected,
        r#"{"results":[{"category_scores":{"prompt_injection":0.9,"jailbreak":null}}]}"#,
    );
}

#[test]
fn lakera_moderation_rejects_a_score_that_is_not_a_number() {
    let body = r#"{"results":[{"category_scores":{"jailbreak":"high"}}]}"#;
    let error = serde_json::from_str::<LakeraModerationResponse>(body)
        .expect_err("a score that is not a number");
    assert!(error.to_string().contains("expected f64"), "{error}");
}

#[test]
fn lakera_moderation_passes_an_empty_result_list() {
    let body = r#"{"results":[]}"#;
    assert!(verdict(None, body).is_none());
    assert!(verdict(Some(thresholds(Some(0.5_f64), Some(0.5_f64))), body).is_none());
}

#[test]
fn lakera_moderation_passes_a_null_result_list() {
    assert!(verdict(None, r#"{"results":null}"#).is_none());
}

#[test]
fn lakera_moderation_reads_only_the_first_result() {
    let clean_first = r#"{"results":[{"flagged":false},{"flagged":true}]}"#;
    assert!(verdict(None, clean_first).is_none());
    let flagged_first = r#"{"results":[{"flagged":true},{"flagged":false}]}"#;
    let rejected = rejection(verdict(None, flagged_first));
    assert_eq!(rejected.error, "Violated content safety policy");
}

#[test]
fn lakera_moderation_reads_an_integer_score() {
    let body = r#"{"results":[{"category_scores":{"jailbreak":1}}]}"#;
    let rejected = rejection(verdict(Some(thresholds(None, Some(1.0_f64))), body));
    assert_eq!(rejected.error, "Violated jailbreak threshold");
}

#[test]
fn lakera_moderation_keeps_unknown_response_keys_in_the_detail() {
    let body = r#"{"model":"lakera-guard","results":[{"flagged":true,"payload":[{"start":0,"end":4}],"category_scores":{"jailbreak":0.9,"moderated_content":0.2}}]}"#;
    let rejected = rejection(verdict(None, body));
    assert_echoed(
        &rejected,
        r#"{"results":[{"flagged":true,"category_scores":{"jailbreak":0.9,"moderated_content":0.2},"payload":[{"start":0,"end":4}]}],"model":"lakera-guard"}"#,
    );
}
