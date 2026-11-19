#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::request_root_paths::Settings as RootSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::types::Config;
use rstest::rstest;
use tower::ServiceExt as _;

use super::checker::{ComplianceChecker, Regulation};
use super::decode::decode_compliance_request;
use super::error::{ComplianceError, Issue, LocPart};
use super::model::{ComplianceRequest, GuardrailMode, GuardrailStatus};
use super::router;
use crate::middleware::request_root_paths::RootPaths;
use crate::routes;
use crate::routes::tests::state_for;

const GENERAL: &str = "general_settings:\n  master_key: oracle-master\n";

fn config() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(GENERAL)
    .unwrap()
}

fn application() -> Router {
    let loaded = config();
    router::routes(&state_for(&loaded))
}

async fn post(application: Router, path: &str, body: &str) -> (StatusCode, String) {
    let request = Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let response = application.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn decode(text: &str) -> ComplianceRequest {
    decode_compliance_request(text.as_bytes()).unwrap()
}

fn issues(text: &str) -> Vec<Issue> {
    let Err(ComplianceError::Request(found)) = decode_compliance_request(text.as_bytes()) else {
        panic!("the decoder should report request issues");
    };
    found
}

#[test]
fn decode_accepts_the_declared_fields_and_arbitrary_logged_fields() {
    let request = decode(
        r#"{"request_id":"req","user_id":"user-1","model":"gpt-4o","timestamp":"2026-01-02T03:04:05Z",
            "guardrail_information":[{"guardrail_status":"success","anything":{"nested":[1,true]}}],
            "extra_top_level":5}"#,
    );
    assert_eq!(request.request_id, "req");
    assert_eq!(request.user_id.as_deref(), Some("user-1"));
    assert_eq!(request.model.as_deref(), Some("gpt-4o"));
    assert_eq!(request.timestamp.as_deref(), Some("2026-01-02T03:04:05Z"));
    assert_eq!(request.guardrail_information.len(), 1);
    assert_eq!(
        request.guardrail_information[0].status,
        GuardrailStatus::Success
    );
    assert_eq!(request.guardrail_information[0].mode, GuardrailMode::Absent);
}

#[test]
fn decode_keeps_nullable_and_empty_evidence() {
    let cleared = decode(
        r#"{"request_id":"r","user_id":null,"model":null,"timestamp":null,
            "guardrail_information":null}"#,
    );
    assert_eq!(cleared.user_id, None);
    assert_eq!(cleared.model, None);
    assert_eq!(cleared.timestamp, None);
    assert!(cleared.guardrail_information.is_empty());

    let empty = decode(r#"{"request_id":"r","user_id":"","model":"","timestamp":""}"#);
    assert_eq!(empty.user_id.as_deref(), Some(""));
    assert_eq!(empty.model.as_deref(), Some(""));
    assert_eq!(empty.timestamp.as_deref(), Some(""));
    assert!(empty.guardrail_information.is_empty());

    let logged = decode(
        r#"{"request_id":"r","guardrail_information":[{"guardrail_status":null,"guardrail_mode":null}]}"#,
    );
    assert_eq!(
        logged.guardrail_information[0].status,
        GuardrailStatus::Other
    );
    assert_eq!(logged.guardrail_information[0].mode, GuardrailMode::Absent);
}

#[rstest]
#[case::missing_request_id(
    r#"{"user_id":"u"}"#,
    "missing",
    vec![LocPart::Name("body"), LocPart::Name("request_id")],
    "Field required"
)]
#[case::null_request_id(
    r#"{"request_id":null}"#,
    "string_type",
    vec![LocPart::Name("body"), LocPart::Name("request_id")],
    "Input should be a valid string"
)]
#[case::numeric_request_id(
    r#"{"request_id":7}"#,
    "string_type",
    vec![LocPart::Name("body"), LocPart::Name("request_id")],
    "Input should be a valid string"
)]
#[case::numeric_user_id(
    r#"{"request_id":"r","user_id":7}"#,
    "string_type",
    vec![LocPart::Name("body"), LocPart::Name("user_id")],
    "Input should be a valid string"
)]
#[case::numeric_model(
    r#"{"request_id":"r","model":7}"#,
    "string_type",
    vec![LocPart::Name("body"), LocPart::Name("model")],
    "Input should be a valid string"
)]
#[case::numeric_timestamp(
    r#"{"request_id":"r","timestamp":7}"#,
    "string_type",
    vec![LocPart::Name("body"), LocPart::Name("timestamp")],
    "Input should be a valid string"
)]
#[case::guardrails_not_a_list(
    r#"{"request_id":"r","guardrail_information":7}"#,
    "list_type",
    vec![LocPart::Name("body"), LocPart::Name("guardrail_information")],
    "Input should be a valid list"
)]
#[case::null_guardrail_entry(
    r#"{"request_id":"r","guardrail_information":[null]}"#,
    "dict_type",
    vec![
        LocPart::Name("body"),
        LocPart::Name("guardrail_information"),
        LocPart::Index(0),
    ],
    "Input should be a valid dictionary"
)]
#[case::numeric_second_guardrail_entry(
    r#"{"request_id":"r","guardrail_information":[{"guardrail_status":"success"},7]}"#,
    "dict_type",
    vec![
        LocPart::Name("body"),
        LocPart::Name("guardrail_information"),
        LocPart::Index(1),
    ],
    "Input should be a valid dictionary"
)]
fn decode_reports_invalid_field_shapes(
    #[case] body: &str,
    #[case] kind: &str,
    #[case] expected_loc: Vec<LocPart>,
    #[case] message: &str,
) {
    let found = issues(body);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, kind);
    assert_eq!(found[0].loc, expected_loc);
    assert_eq!(found[0].msg, message);
}

#[rstest]
#[case("[]")]
#[case(r#""text""#)]
#[case("null")]
fn decode_rejects_a_non_object_body(#[case] body: &str) {
    let found = issues(body);
    assert_eq!(found[0].kind, "model_attributes_type");
    assert_eq!(found[0].loc, vec![LocPart::Name("body")]);
}

#[test]
fn decode_rejects_malformed_and_empty_bodies() {
    let malformed = issues("{");
    assert_eq!(malformed[0].kind, "json_invalid");
    let missing = issues("");
    assert_eq!(missing[0].kind, "missing");
    assert_eq!(missing[0].loc, vec![LocPart::Name("body")]);
}

#[rstest]
#[case::missing(r#"{"logged":true}"#, GuardrailStatus::Other)]
#[case::null(r#"{"guardrail_status":null}"#, GuardrailStatus::Other)]
#[case::not_run(r#"{"guardrail_status":"not_run"}"#, GuardrailStatus::NotRun)]
#[case::success(r#"{"guardrail_status":"success"}"#, GuardrailStatus::Success)]
#[case::intervened(
    r#"{"guardrail_status":"guardrail_intervened"}"#,
    GuardrailStatus::Intervened
)]
#[case::failed(r#"{"guardrail_status":"failed"}"#, GuardrailStatus::Failed)]
#[case::blocked(r#"{"guardrail_status":"blocked"}"#, GuardrailStatus::Blocked)]
#[case::other_text(r#"{"guardrail_status":"weird"}"#, GuardrailStatus::Other)]
#[case::other_type(r#"{"guardrail_status":false}"#, GuardrailStatus::Other)]
fn decode_classifies_guardrail_status(#[case] entry: &str, #[case] expected: GuardrailStatus) {
    let body = format!(r#"{{"request_id":"r","guardrail_information":[{entry}]}}"#);
    let request = decode(&body);
    assert_eq!(request.guardrail_information[0].status, expected);
}

#[rstest]
#[case::absent(None, true)]
#[case::null(Some("null"), true)]
#[case::pre_call(Some(r#""pre_call""#), true)]
#[case::post_call(Some(r#""post_call""#), false)]
#[case::empty_list(Some("[]"), false)]
#[case::single_pre_call(Some(r#"["pre_call"]"#), true)]
#[case::single_post_call(Some(r#"["post_call"]"#), false)]
#[case::mixed_list(Some(r#"["pre_call","post_call"]"#), false)]
#[case::repeated_pre_call(Some(r#"["pre_call","pre_call"]"#), true)]
#[case::nested_list(Some(r#"[["pre_call"]]"#), false)]
#[case::default_only(Some(r#"{"default":"pre_call"}"#), true)]
#[case::null_default(Some(r#"{"default":null}"#), false)]
#[case::missing_default(Some("{}"), false)]
#[case::tags_without_default(Some(r#"{"tags":{"a":"pre_call"}}"#), false)]
#[case::agreeing_tags(Some(r#"{"default":"pre_call","tags":{"a":"pre_call"}}"#), true)]
#[case::divergent_tags(Some(r#"{"default":"pre_call","tags":{"a":"post_call"}}"#), false)]
#[case::tags_not_a_dictionary(Some(r#"{"default":"pre_call","tags":"oops"}"#), true)]
#[case::empty_tag_branch(Some(r#"{"default":"pre_call","tags":{"a":[]}}"#), false)]
#[case::list_tag_branch(Some(r#"{"default":"pre_call","tags":{"a":["pre_call"]}}"#), true)]
#[case::wrong_list_tag_branch(Some(r#"{"default":"pre_call","tags":{"a":["post_call"]}}"#), false)]
#[case::numeric_tag_branch(Some(r#"{"default":"pre_call","tags":{"a":7}}"#), false)]
#[case::list_default(Some(r#"{"default":["pre_call"]}"#), true)]
#[case::empty_list_default(Some(r#"{"default":[]}"#), false)]
#[case::numeric_default(Some(r#"{"default":7}"#), false)]
#[case::numeric_mode(Some("7"), false)]
#[case::boolean_mode(Some("true"), false)]
fn mode_guarantees_only_a_matching_pre_call(
    #[case] fragment: Option<&str>,
    #[case] expected: bool,
) {
    let value = fragment.map(|text| serde_json::from_str::<JsonValue>(text).unwrap());
    let mode = GuardrailMode::from_value(value.as_ref());
    assert_eq!(mode.matches("pre_call"), expected);
}

#[test]
fn mode_matching_is_exact_for_other_modes() {
    let value: JsonValue = serde_json::from_str(r#""post_call""#).unwrap();
    let mode = GuardrailMode::from_value(Some(&value));
    assert!(mode.matches("post_call"));
    assert!(!mode.matches("pre_call"));
}

#[test]
fn the_checker_discards_only_not_run_evidence() {
    let request = decode(
        r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{"guardrail_status":"not_run"},{"guardrail_status":"success"},
                {"logged":"x"}]}"#,
    );
    let checks = ComplianceChecker::new(&request)
        .response(Regulation::EuAiAct)
        .checks;
    assert!(checks[0].passed);
    assert_eq!(checks[0].detail, "2 guardrail(s) applied");
}

#[test]
fn eu_ai_act_returns_the_three_articles_in_order() {
    let request =
        decode(r#"{"request_id":"r","guardrail_information":[{"guardrail_status":"success"}]}"#);
    let response = ComplianceChecker::new(&request).response(Regulation::EuAiAct);
    assert_eq!(response.regulation, "EU AI Act");
    assert!(!response.compliant);
    let articles = response
        .checks
        .iter()
        .map(|check| (check.article, check.check_name))
        .collect::<Vec<_>>();
    assert_eq!(
        articles,
        vec![
            ("Art. 9", "Guardrails applied"),
            ("Art. 5", "Content screened before LLM"),
            ("Art. 12", "Audit record complete"),
        ]
    );
    assert_eq!(response.checks[0].detail, "1 guardrail(s) applied");
    assert_eq!(
        response.checks[1].detail,
        "1 pre-call guardrail(s) screened content"
    );
    assert_eq!(
        response.checks[2].detail,
        "Missing: user_id, model, timestamp"
    );
}

#[test]
fn gdpr_returns_the_three_articles_in_order() {
    let request = decode(
        r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{"guardrail_status":"success"}]}"#,
    );
    let response = ComplianceChecker::new(&request).response(Regulation::Gdpr);
    assert_eq!(response.regulation, "GDPR");
    assert!(response.compliant);
    let articles = response
        .checks
        .iter()
        .map(|check| check.article)
        .collect::<Vec<_>>();
    assert_eq!(articles, vec!["Art. 32", "Art. 5(1)(c)", "Art. 30"]);
    assert_eq!(
        response.checks[0].detail,
        "1 pre-call guardrail(s) protect data"
    );
    assert_eq!(response.checks[1].detail, "No sensitive data detected");
    assert_eq!(
        response.checks[2].detail,
        "All required audit fields present"
    );
}

#[rstest]
#[case("guardrail_intervened")]
#[case("failed")]
#[case("blocked")]
fn sensitive_protection_passes_on_every_intervention_status(#[case] status: &str) {
    let body = format!(
        r#"{{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{{"guardrail_status":"{status}","guardrail_mode":"pre_call"}}]}}"#
    );
    let request = decode(&body);
    let checks = ComplianceChecker::new(&request)
        .response(Regulation::Gdpr)
        .checks;
    assert!(checks[1].passed);
    assert_eq!(
        checks[1].detail,
        "Guardrail intervened to protect sensitive data"
    );
}

#[test]
fn an_intervention_wins_over_other_successful_pre_call_guardrails() {
    let request = decode(
        r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{"guardrail_status":"success","guardrail_mode":"pre_call"},
                {"guardrail_status":"failed","guardrail_mode":"pre_call"}]}"#,
    );
    let checks = ComplianceChecker::new(&request)
        .response(Regulation::Gdpr)
        .checks;
    assert_eq!(
        checks[1].detail,
        "Guardrail intervened to protect sensitive data"
    );
}

#[rstest]
#[case::post_call_only(
    r#"{"guardrail_status":"success","guardrail_mode":"post_call"}"#,
    false,
    "No pre-call guardrails to protect sensitive data"
)]
#[case::mixed_statuses(
    r#"{"guardrail_status":"success","guardrail_mode":"pre_call"},
        {"guardrail_status":"weird","guardrail_mode":"pre_call"}"#,
    false,
    "No pre-call guardrails to protect sensitive data"
)]
#[case::all_success(
    r#"{"guardrail_status":"success","guardrail_mode":"pre_call"}"#,
    true,
    "No sensitive data detected"
)]
#[case::no_evidence("", false, "No pre-call guardrails to protect sensitive data")]
fn sensitive_protection_needs_a_nonempty_successful_pre_call_set(
    #[case] entries: &str,
    #[case] passed: bool,
    #[case] detail: &str,
) {
    let body = format!(
        r#"{{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{entries}]}}"#
    );
    let request = decode(&body);
    let checks = ComplianceChecker::new(&request)
        .response(Regulation::Gdpr)
        .checks;
    assert_eq!(checks[1].passed, passed);
    assert_eq!(checks[1].detail, detail);
}

#[rstest]
#[case::all_present(
    r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
        "guardrail_information":[{"guardrail_status":"success"}]}"#,
    true,
    "All required audit fields present"
)]
#[case::everything_missing(
    r#"{"request_id":"r"}"#,
    false,
    "Missing: user_id, model, timestamp, guardrail_results"
)]
#[case::guardrails_missing(
    r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t"}"#,
    false,
    "Missing: guardrail_results"
)]
#[case::empty_strings(
    r#"{"request_id":"r","user_id":"","model":"","timestamp":"",
        "guardrail_information":[{"guardrail_status":"success"}]}"#,
    false,
    "Missing: user_id, model, timestamp"
)]
#[case::arbitrary_timestamp(
    r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"not a date",
        "guardrail_information":[{"guardrail_status":"success"}]}"#,
    true,
    "All required audit fields present"
)]
#[case::null_model(
    r#"{"request_id":"r","user_id":"u","model":null,"timestamp":"t",
        "guardrail_information":[{"guardrail_status":"success"}]}"#,
    false,
    "Missing: model"
)]
fn audit_completeness_lists_the_source_fields_in_order(
    #[case] body: &str,
    #[case] passed: bool,
    #[case] detail: &str,
) {
    let request = decode(body);
    let checks = ComplianceChecker::new(&request)
        .response(Regulation::EuAiAct)
        .checks;
    assert_eq!(checks[2].passed, passed);
    assert_eq!(checks[2].detail, detail);
    let gdpr = ComplianceChecker::new(&request)
        .response(Regulation::Gdpr)
        .checks;
    assert_eq!(gdpr[2].detail, detail);
}

#[test]
fn a_response_is_compliant_only_when_every_check_passes() {
    let passing = decode(
        r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{"guardrail_status":"success"}]}"#,
    );
    assert!(
        ComplianceChecker::new(&passing)
            .response(Regulation::EuAiAct)
            .compliant
    );

    let missing_screening = decode(
        r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{"guardrail_status":"success","guardrail_mode":"post_call"}]}"#,
    );
    let response = ComplianceChecker::new(&missing_screening).response(Regulation::EuAiAct);
    assert!(!response.compliant);
    assert!(response.checks[0].passed);
    assert!(!response.checks[1].passed);
    assert!(response.checks[2].passed);
}

#[tokio::test]
async fn both_routes_answer_ordered_article_results() {
    let app = application();
    let (status, text) = post(
        app.clone(),
        "/compliance/eu-ai-act",
        r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{"guardrail_status":"success"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(body["regulation"], "EU AI Act");
    assert_eq!(body["compliant"], true);
    assert_eq!(body["checks"][0]["check_name"], "Guardrails applied");
    assert_eq!(body["checks"][1]["article"], "Art. 5");
    assert_eq!(body["checks"][2]["article"], "Art. 12");

    let (status, text) = post(
        app,
        "/compliance/gdpr",
        r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{"guardrail_status":"success"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(body["regulation"], "GDPR");
    assert_eq!(body["compliant"], true);
    assert_eq!(body["checks"][1]["article"], "Art. 5(1)(c)");
    assert_eq!(body["checks"][2]["article"], "Art. 30");
}

#[tokio::test]
async fn a_root_prefix_still_selects_the_gdpr_articles() {
    let loaded = config();
    let application = router::routes(&state_for(&loaded));
    let roots = RootSettings {
        server_root_path: String::new(),
        server_root_paths: vec!["/tenant-a".to_owned()],
    };
    let wrapped = RootPaths::from_settings(&roots).unwrap().wrap(application);
    let (status, text) = post(
        wrapped,
        "/tenant-a/compliance/gdpr",
        r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t",
            "guardrail_information":[{"guardrail_status":"success"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(body["regulation"], "GDPR");
    assert_eq!(body["checks"][0]["article"], "Art. 32");
    assert_eq!(body["checks"][1]["article"], "Art. 5(1)(c)");
}

#[tokio::test]
async fn a_negative_verdict_is_an_ordinary_200_response() {
    let (status, text) = post(
        application(),
        "/compliance/eu-ai-act",
        r#"{"request_id":"r","guardrail_information":[{"guardrail_status":"success"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(body["compliant"], false);
    assert_eq!(
        body["checks"][2]["detail"],
        "Missing: user_id, model, timestamp"
    );
}

#[tokio::test]
async fn invalid_shapes_return_the_fastapi_error_envelope() {
    let (status, text) = post(application(), "/compliance/eu-ai-act", r#"{"user_id":"u"}"#).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        text,
        r#"{"detail":[{"type":"missing","loc":["body","request_id"],"msg":"Field required"}]}"#
    );

    let (status, text) = post(
        application(),
        "/compliance/gdpr",
        r#"{"request_id":"r","guardrail_information":[null]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        text,
        r#"{"detail":[{"type":"dict_type","loc":["body","guardrail_information",0],"msg":"Input should be a valid dictionary"}]}"#
    );
}

#[tokio::test]
async fn a_non_object_body_is_rejected_instead_of_failing_inside_auth() {
    let (status, text) = post(application(), "/compliance/eu-ai-act", "[]").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        text,
        r#"{"detail":[{"type":"model_attributes_type","loc":["body"],"msg":"Input should be a valid dictionary or object to extract fields from"}]}"#
    );
}

#[tokio::test]
async fn the_application_mounts_both_routes_behind_the_key_check() {
    let loaded = config();
    let application = routes::router(
        state_for(&loaded),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    );
    let authorized = Request::post("/compliance/eu-ai-act")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, "Bearer oracle-master")
        .body(Body::from(
            r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t"}"#,
        ))
        .unwrap();
    let response = application.clone().oneshot(authorized).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let anonymous = Request::post("/compliance/gdpr")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"request_id":"r"}"#))
        .unwrap();
    let response = application.oneshot(anonymous).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[rstest]
fn replays_the_compliance_request_corpus(
    #[files("fuzz/corpus/compliance_request/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("a corpus seed should be readable");
    let first = decode_compliance_request(&bytes);
    let second = decode_compliance_request(&bytes);
    match (first, second) {
        (Ok(left), Ok(right)) => assert_eq!(left, right),
        (Err(_), Err(_)) => {}
        (Ok(_), Err(_)) | (Err(_), Ok(_)) => {
            panic!("the decoder answered differently for the same bytes");
        }
    }
}
