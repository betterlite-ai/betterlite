use betterlite_config::sections::guardrail_params::records::LakeraCategoryThresholds;
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::json;

use crate::lakera::rejection::{LakeraRejection, LakeraRejectionDetail};
use crate::lakera::response::LakeraModerationResponse;
use crate::lakera::{LegacyLakeraModeration, LegacyLakeraModerationParams};

fn moderation(thresholds: Option<LakeraCategoryThresholds>) -> LegacyLakeraModeration {
    LegacyLakeraModeration::new(
        LegacyLakeraModerationParams {
            category_thresholds: thresholds,
            ..LegacyLakeraModerationParams::default()
        },
        |_key| String::new(),
        reqwest::Client::default,
    )
}

fn thresholds(
    prompt_injection: Option<f64>,
    jailbreak: Option<f64>,
) -> Option<LakeraCategoryThresholds> {
    Some(LakeraCategoryThresholds {
        prompt_injection,
        jailbreak,
    })
}

fn parse(body: impl Serialize) -> LakeraModerationResponse {
    let value = serde_json::to_value(body).expect("the fixture is JSON");
    serde_json::from_value(value).expect("the Lakera response parses")
}

fn rejection(rejected: Option<LakeraRejection>) -> LakeraRejectionDetail {
    let rejection = rejected.expect("the response is rejected");
    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
    rejection.detail
}

fn assert_echoed(detail: &LakeraRejectionDetail, expected: impl Serialize) {
    let written = serde_json::to_value(&detail.lakera_ai_response).expect("the response is JSON");
    assert_eq!(
        written,
        serde_json::to_value(expected).expect("the fixture is JSON")
    );
}

#[test]
fn lakera_moderation_blocks_at_the_jailbreak_threshold() {
    let response = parse(json!({"results": [{"category_scores": {"jailbreak": 0.8}}]}));
    let rejected =
        rejection(moderation(thresholds(None, Some(0.8))).check_response_flagged(&response));
    assert_eq!(rejected.error, "Violated jailbreak threshold");
    assert_echoed(
        &rejected,
        json!({"results": [{"category_scores": {"jailbreak": 0.8}}]}),
    );
}

#[test]
fn lakera_moderation_passes_below_the_jailbreak_threshold() {
    let response = parse(json!({"results": [{"category_scores": {"jailbreak": 0.79}}]}));
    assert!(
        moderation(thresholds(None, Some(0.8)))
            .check_response_flagged(&response)
            .is_none()
    );
}

#[test]
fn lakera_moderation_blocks_at_the_prompt_injection_threshold() {
    let response = parse(json!({"results": [{"category_scores": {"prompt_injection": 0.5}}]}));
    let rejected =
        rejection(moderation(thresholds(Some(0.5), None)).check_response_flagged(&response));
    assert_eq!(rejected.error, "Violated prompt_injection threshold");
}

#[test]
fn lakera_moderation_passes_below_the_prompt_injection_threshold() {
    let response = parse(json!({"results": [{"category_scores": {"prompt_injection": 0.49}}]}));
    assert!(
        moderation(thresholds(Some(0.5), None))
            .check_response_flagged(&response)
            .is_none()
    );
}

#[test]
fn lakera_moderation_reports_a_jailbreak_before_a_prompt_injection() {
    let response = parse(
        json!({"results": [{"category_scores": {"jailbreak": 0.9, "prompt_injection": 0.9}}]}),
    );
    let rejected =
        rejection(moderation(thresholds(Some(0.5), Some(0.5))).check_response_flagged(&response));
    assert_eq!(rejected.error, "Violated jailbreak threshold");
}

#[test]
fn lakera_moderation_passes_when_the_breaching_score_has_no_threshold() {
    let response = parse(
        json!({"results": [{"category_scores": {"jailbreak": 0.99, "prompt_injection": 0.1}}]}),
    );
    assert!(
        moderation(thresholds(Some(0.5), None))
            .check_response_flagged(&response)
            .is_none()
    );
}

#[test]
fn lakera_moderation_passes_when_the_threshold_has_no_score() {
    let response = parse(json!({"results": [{"category_scores": {"prompt_injection": 0.1}}]}));
    assert!(
        moderation(thresholds(Some(0.5), Some(0.5)))
            .check_response_flagged(&response)
            .is_none()
    );
}

#[test]
fn lakera_moderation_ignores_flagged_when_thresholds_have_no_scores() {
    let response = parse(json!({"results": [{"flagged": true}]}));
    assert!(
        moderation(thresholds(Some(0.5), Some(0.5)))
            .check_response_flagged(&response)
            .is_none()
    );
}

#[test]
fn lakera_moderation_ignores_an_empty_score_object() {
    let response = parse(json!({"results": [{"flagged": true, "category_scores": {}}]}));
    assert!(
        moderation(thresholds(Some(0.5), Some(0.5)))
            .check_response_flagged(&response)
            .is_none()
    );
}

#[test]
fn lakera_moderation_blocks_a_flagged_response_without_thresholds() {
    let response =
        parse(json!({"results": [{"flagged": true, "category_scores": {"jailbreak": 0.1}}]}));
    let rejected = rejection(moderation(None).check_response_flagged(&response));
    assert_eq!(rejected.error, "Violated content safety policy");
    assert_echoed(
        &rejected,
        json!({"results": [{"flagged": true, "category_scores": {"jailbreak": 0.1}}]}),
    );
}

#[test]
fn lakera_moderation_passes_an_unflagged_response_without_thresholds() {
    let response = parse(json!({"results": [{"flagged": false}]}));
    assert!(moderation(None).check_response_flagged(&response).is_none());
}

#[test]
fn lakera_moderation_passes_a_response_without_a_flagged_key() {
    let response = parse(json!({"results": [{"category_scores": {"jailbreak": 0.9}}]}));
    assert!(moderation(None).check_response_flagged(&response).is_none());
}

#[test]
fn lakera_moderation_passes_a_null_flagged_value() {
    let response = parse(json!({"results": [{"flagged": null}]}));
    assert!(moderation(None).check_response_flagged(&response).is_none());
}

#[test]
fn lakera_moderation_passes_a_null_category_scores_map() {
    let response = parse(json!({"results": [{"flagged": true, "category_scores": null}]}));
    assert!(
        moderation(thresholds(Some(0.5), Some(0.5)))
            .check_response_flagged(&response)
            .is_none()
    );
}

#[test]
fn lakera_moderation_keeps_a_null_score_in_the_detail() {
    let body =
        json!({"results": [{"category_scores": {"jailbreak": null, "prompt_injection": 0.9}}]});
    let response = parse(body.clone());
    let rejected =
        rejection(moderation(thresholds(Some(0.5), None)).check_response_flagged(&response));
    assert_echoed(&rejected, &body);
}

#[test]
fn lakera_moderation_rejects_a_score_that_is_not_a_number() {
    let error = serde_json::from_value::<LakeraModerationResponse>(
        json!({"results": [{"category_scores": {"jailbreak": "high"}}]}),
    )
    .expect_err("a score that is not a number");
    assert!(error.to_string().contains("expected f64"), "{error}");
}

#[test]
fn lakera_moderation_passes_an_empty_result_list() {
    let response = parse(json!({"results": []}));
    assert!(moderation(None).check_response_flagged(&response).is_none());
    assert!(
        moderation(thresholds(Some(0.5), Some(0.5)))
            .check_response_flagged(&response)
            .is_none()
    );
}

#[test]
fn lakera_moderation_passes_a_null_result_list() {
    let response = parse(json!({"results": null}));
    assert!(moderation(None).check_response_flagged(&response).is_none());
}

#[test]
fn lakera_moderation_reads_only_the_first_result() {
    let response = parse(json!({
        "results": [
            {"flagged": false},
            {"flagged": true},
        ]
    }));
    assert!(moderation(None).check_response_flagged(&response).is_none());
    let response = parse(json!({
        "results": [
            {"flagged": true},
            {"flagged": false},
        ]
    }));
    let rejected = rejection(moderation(None).check_response_flagged(&response));
    assert_eq!(rejected.error, "Violated content safety policy");
}

#[test]
fn lakera_moderation_reads_an_integer_score() {
    let response = parse(json!({"results": [{"category_scores": {"jailbreak": 1}}]}));
    let rejected =
        rejection(moderation(thresholds(None, Some(1.0))).check_response_flagged(&response));
    assert_eq!(rejected.error, "Violated jailbreak threshold");
}

#[test]
fn lakera_moderation_keeps_unknown_response_keys_in_the_detail() {
    let body = json!({
        "model": "lakera-guard",
        "results": [{
            "flagged": true,
            "payload": [{"start": 0, "end": 4}],
            "category_scores": {"jailbreak": 0.9, "moderated_content": 0.2},
        }],
    });
    let response = parse(body.clone());
    let rejected = rejection(moderation(None).check_response_flagged(&response));
    assert_echoed(&rejected, &body);
}
