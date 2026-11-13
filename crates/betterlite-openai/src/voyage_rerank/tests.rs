#![cfg(test)]

use std::{error::Error as _, fs, path::PathBuf};

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    RerankLogging, VoyageRerankBilledUnits, VoyageRerankControls, VoyageRerankCostError,
    VoyageRerankError, VoyageRerankResult, api_key, calculate_rerank_cost, decode_voyage_rerank,
    endpoint, headers, request,
};
use crate::api::rerank_contracts::records::RerankDocument;

const INJECTED: Uuid = Uuid::from_u128(7);

struct Recorder {
    calls: usize,
}

impl RerankLogging for Recorder {
    fn post_call(&mut self, _original_response: &[u8]) {
        self.calls += 1;
    }
}

fn decode(
    body: &str,
    status: u16,
) -> (
    Result<super::VoyageRerankResponse, VoyageRerankError>,
    usize,
) {
    let mut recorder = Recorder { calls: 0 };
    let result = decode_voyage_rerank(body.as_bytes(), status, INJECTED, &mut recorder);
    (result, recorder.calls)
}

fn key_error(primary: Option<&str>, secondary: Option<&str>) -> Result<String, VoyageRerankError> {
    api_key(None, |name| match name {
        "VOYAGE_API_KEY" => primary.map(str::to_owned),
        "VOYAGE_AI_API_KEY" => secondary.map(str::to_owned),
        "VOYAGE_AI_TOKEN" => Some("third".to_owned()),
        _ => None,
    })
}

#[rstest::rstest]
#[case::primary(Some("first"), None, "first")]
#[case::secondary(None, Some("second"), "second")]
#[case::precedence(Some("first"), Some("second"), "first")]
#[case::empty_primary_falls_through(Some(""), Some("second"), "second")]
#[case::empty_secondary(Some(""), Some(""), "")]
fn key_resolution_uses_only_the_two_aliases(
    #[case] primary: Option<&str>,
    #[case] secondary: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(key_error(primary, secondary).expect("key"), expected);
}

#[test]
fn token_alias_alone_resolves_to_no_key() {
    let error = api_key(None, |name| {
        (name == "VOYAGE_AI_TOKEN").then(|| "third".to_owned())
    })
    .expect_err("token alias is ignored");
    assert!(matches!(error, VoyageRerankError::MissingApiKey));
}

#[test]
fn explicit_empty_key_is_accepted() {
    assert_eq!(api_key(Some(""), |_| None).expect("explicit"), "");
}

#[test]
fn explicit_key_skips_the_environment() {
    let key = api_key(Some("supplied"), |_| Some("env".to_owned())).expect("key");
    assert_eq!(key, "supplied");
}

#[test]
fn caller_headers_are_discarded() {
    let supplied = serde_json::from_str(r#"{"X-Caller": "discard", "Authorization": "old"}"#)
        .expect("headers");
    let built = headers(supplied, "key");
    assert_eq!(built.len(), 2);
    assert_eq!(built["Authorization"], "Bearer key");
    assert_eq!(built["content-type"], "application/json");
}

#[rstest::rstest]
#[case::default(None, "https://api.voyageai.com/v1/rerank")]
#[case::supplied("https://example.test", "https://example.test/v1/rerank")]
#[case::v1("https://example.test/v1", "https://example.test/v1/rerank")]
#[case::complete("https://example.test/v1/rerank", "https://example.test/v1/rerank")]
#[case::trailing("https://example.test/v1/", "https://example.test/v1/rerank")]
#[case::double_trailing("https://example.test/v1//", "https://example.test/v1/rerank")]
#[case::empty("", "/v1/rerank")]
#[case::slash("/", "/v1/rerank")]
fn endpoints_trim_and_select_the_suffix(#[case] base: Option<&str>, #[case] expected: &str) {
    assert_eq!(endpoint(base), expected);
}

fn documents() -> Vec<RerankDocument> {
    vec![
        RerankDocument::Text("first".to_owned()),
        RerankDocument::Object(indexmap::IndexMap::from([(
            "text".to_owned(),
            Json::Text("second".to_owned()),
        )])),
    ]
}

#[test]
fn top_n_becomes_top_k_and_documents_are_kept() {
    let controls = VoyageRerankControls {
        top_n: Some(3),
        ..VoyageRerankControls::default()
    };
    let body = request("m", "q", &documents(), &controls);
    assert_eq!(
        serde_json::to_value(&body).expect("body"),
        json!({"model": "m", "query": "q", "documents": ["first", {"text": "second"}], "top_k": 3})
    );
}

#[rstest::rstest]
#[case::yes(Some(true), json!(true))]
#[case::no(Some(false), json!(false))]
fn return_documents_is_kept_when_present(#[case] value: Option<bool>, #[case] expected: Value) {
    let controls = VoyageRerankControls {
        return_documents: value,
        ..VoyageRerankControls::default()
    };
    let body = request("m", "q", &documents(), &controls);
    assert_eq!(
        serde_json::to_value(&body).expect("body")["return_documents"],
        expected
    );
}

#[test]
fn none_controls_are_omitted() {
    let body = request("m", "q", &documents(), &VoyageRerankControls::default());
    let value = serde_json::to_value(&body).expect("body");
    assert!(value.get("top_k").is_none());
    assert!(value.get("return_documents").is_none());
}

#[test]
fn other_declared_controls_are_ignored() {
    let controls = VoyageRerankControls {
        rank_fields: Some(vec!["title".to_owned()]),
        max_chunks_per_doc: Some(4),
        max_tokens_per_doc: Some(8),
        instruction: Some("rank".to_owned()),
        ..VoyageRerankControls::default()
    };
    let body = request("m", "q", &documents(), &controls);
    let value = serde_json::to_value(&body).expect("body");
    assert_eq!(
        value.as_object().expect("object").len(),
        3,
        "only model, query and documents reach the body"
    );
}

#[test]
fn status_check_runs_before_the_log_hook() {
    let (result, calls) = decode(r#"{"data": []}"#, 201);
    assert!(matches!(
        result,
        Err(VoyageRerankError::Status { status: 201 })
    ));
    assert_eq!(calls, 0);
}

#[test]
fn parse_failure_keeps_the_status_and_logs_first() {
    let (result, calls) = decode("not-json", 200);
    match result {
        Err(VoyageRerankError::Decode {
            status, message, ..
        }) => {
            assert_eq!(status, 200);
            assert_eq!(message, "not-json");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(calls, 1);
}

#[rstest::rstest]
#[case::missing("{}")]
#[case::null(r#"{"data": null}"#)]
fn absent_or_null_data_is_rejected(#[case] body: &str) {
    let (result, calls) = decode(body, 200);
    assert!(matches!(result, Err(VoyageRerankError::NoResults)));
    assert_eq!(calls, 1);
}

#[test]
fn results_keep_scores_and_convert_string_documents() {
    let body = r#"{"data":[{"index":1,"relevance_score":0.50,"document":"doc"},{"index":2,"relevance_score":1}]}"#;
    let (result, _) = decode(body, 200);
    let response = result.expect("response");
    assert_eq!(response.id, INJECTED.to_string());
    assert_eq!(
        response.results,
        vec![
            VoyageRerankResult {
                index: 1,
                relevance_score: Decimal::new(5, 1),
                document: Some(Json::Object(indexmap::IndexMap::from([(
                    "text".to_owned(),
                    Json::Text("doc".to_owned()),
                )]))),
            },
            VoyageRerankResult {
                index: 2,
                relevance_score: Decimal::ONE,
                document: None,
            },
        ]
    );
}

#[test]
fn nonstring_documents_are_preserved() {
    let body = r#"{"data":[
        {"index":0,"relevance_score":0.25,"document":{"text":"x","extra":1}},
        {"index":1,"relevance_score":0.5,"document":null},
        {"index":2,"relevance_score":0.75,"document":4}
    ]}"#;
    let response = decode(body, 200).0.expect("response");
    let encoded = serde_json::to_value(&response).expect("reply");
    assert_eq!(
        encoded["results"][0]["document"],
        json!({"text": "x", "extra": 1})
    );
    assert_eq!(encoded["results"][1]["document"], Value::Null);
    assert_eq!(encoded["results"][2]["document"], json!(4));
}

#[rstest::rstest]
#[case::missing(r#"{"data":[{"index":0}]}"#, "relevance_score")]
#[case::wrong_index(r#"{"data":[{"index":"0","relevance_score":1}]}"#, "index")]
#[case::wrong_data(r#"{"data":{}}"#, "data")]
#[case::wrong_item(r#"{"data":[7]}"#, "data[]")]
fn malformed_results_name_their_field(#[case] body: &str, #[case] field: &'static str) {
    let (result, _) = decode(body, 200);
    assert!(matches!(
        result,
        Err(VoyageRerankError::ResponseField { field: found }) if found == field
    ));
}

#[test]
fn absent_usage_defaults_to_zero_tokens() {
    let response = decode(r#"{"data":[]}"#, 200).0.expect("response");
    assert_eq!(response.meta.billed_units.total_tokens, Some(0));
    assert_eq!(response.meta.tokens.input_tokens, Some(0));
    assert_eq!(response.meta.tokens.output_tokens, 0);
}

#[test]
fn usage_tokens_fill_both_meta_fields() {
    let response = decode(r#"{"data":[],"usage":{"total_tokens":7}}"#, 200)
        .0
        .expect("response");
    assert_eq!(response.meta.billed_units.total_tokens, Some(7));
    assert_eq!(response.meta.tokens.input_tokens, Some(7));
    assert_eq!(response.meta.tokens.output_tokens, 0);
}

#[test]
fn null_total_tokens_stay_null() {
    let response = decode(r#"{"data":[],"usage":{"total_tokens":null}}"#, 200)
        .0
        .expect("response");
    assert_eq!(response.meta.billed_units.total_tokens, None);
    assert_eq!(response.meta.tokens.input_tokens, None);
    assert_eq!(response.meta.tokens.output_tokens, 0);
}

#[rstest::rstest]
#[case::fraction(r#"{"data":[],"usage":{"total_tokens":1.5}}"#)]
#[case::text(r#"{"data":[],"usage":{"total_tokens":"7"}}"#)]
fn non_integer_total_tokens_are_rejected(#[case] body: &str) {
    let (result, _) = decode(body, 200);
    assert!(matches!(
        result,
        Err(VoyageRerankError::ResponseField {
            field: "usage.total_tokens"
        })
    ));
}

#[rstest::rstest]
#[case::absent(r#"{"data":[]}"#)]
#[case::empty(r#"{"data":[],"id":""}"#)]
#[case::null(r#"{"data":[],"id":null}"#)]
#[case::zero(r#"{"data":[],"id":0}"#)]
#[case::no(r#"{"data":[],"id":false}"#)]
#[case::empty_list(r#"{"data":[],"id":[]}"#)]
#[case::empty_object(r#"{"data":[],"id":{}}"#)]
fn falsey_ids_use_the_injected_uuid(#[case] body: &str) {
    let response = decode(body, 200).0.expect("response");
    assert_eq!(response.id, INJECTED.to_string());
}

#[test]
fn truthy_ids_are_preserved() {
    let response = decode(r#"{"data":[],"id":"given"}"#, 200)
        .0
        .expect("response");
    assert_eq!(response.id, "given");
}

#[rstest::rstest]
#[case::missing_price(None, Some(VoyageRerankBilledUnits { total_tokens: Some(7) }), Decimal::ZERO)]
#[case::missing_units(Some(Decimal::new(5, 1)), None, Decimal::ZERO)]
#[case::null_tokens(Some(Decimal::new(5, 1)), Some(VoyageRerankBilledUnits { total_tokens: None }), Decimal::ZERO)]
#[case::priced(Some(Decimal::new(5, 1)), Some(VoyageRerankBilledUnits { total_tokens: Some(7) }), Decimal::new(35, 1))]
fn cost_needs_both_a_price_and_billed_tokens(
    #[case] price: Option<Decimal>,
    #[case] units: Option<VoyageRerankBilledUnits>,
    #[case] expected: Decimal,
) {
    let (total, completion) = calculate_rerank_cost(price, units.as_ref()).expect("cost");
    assert_eq!(total, expected);
    assert_eq!(completion, Decimal::ZERO);
}

#[test]
fn cost_overflow_is_typed() {
    let units = VoyageRerankBilledUnits {
        total_tokens: Some(2),
    };
    let result = calculate_rerank_cost(Some(Decimal::MAX), Some(&units));
    assert!(matches!(result, Err(VoyageRerankCostError::Overflow)));
}

#[test]
fn decode_failure_retains_its_json_source() {
    let error = decode("nope", 200).0.expect_err("decode");
    assert!(error.source().is_some());
}

#[rstest::rstest]
fn replays_the_voyage_rerank_corpus(#[files("fuzz/corpus/voyage_rerank/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus file");
    let mut recorder = Recorder { calls: 0 };
    let _unused = decode_voyage_rerank(&bytes, 200, INJECTED, &mut recorder);
}
