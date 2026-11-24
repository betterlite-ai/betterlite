#![cfg(test)]

use betterlite_error::error::ErrorClass;
use pyo3_async_runtimes::tokio::get_runtime;
use rstest::rstest;
use rust_decimal::Decimal;

use super::SessionCore;
use super::logging::logging_state;
use crate::search_ocr::convert::{CostAnswer, CostCall, OcrCall, SearchCall};
use crate::search_ocr::error::NativeFailure;

fn session() -> SessionCore {
    SessionCore::build(r#"{"environment": {}}"#).expect("the session builds")
}

fn cost(text: &str) -> String {
    let call: CostCall = serde_json::from_str(text).expect("the cost call parses");
    get_runtime()
        .block_on(session().cost(call))
        .expect("the cost answers")
}

#[rstest]
#[case::positive(2_i64, Decimal::new(6_i64, 3), Decimal::ZERO)]
#[case::negative(-2_i64, Decimal::new(-6_i64, 3), -Decimal::ZERO)]
fn cost_charges_each_query_and_keeps_the_sign(
    #[case] queries: i64,
    #[case] charge: Decimal,
    #[case] fee: Decimal,
) {
    let text = format!(r#"{{"model": "dataforseo/search", "number_of_queries": {queries}}}"#);
    let answer = cost(&text);
    let encoded = serde_json::to_string(&CostAnswer::Cost {
        input_cost: charge,
        output_cost: fee,
    })
    .expect("the answer encodes");
    assert_eq!(answer, encoded);
}

// litellm returns parallel_ai's price before multiplying, so the count's sign must not reach it
#[test]
fn cost_keeps_the_parallel_ai_price_when_the_count_is_negative() {
    let text = r#"{"model": "parallel_ai/search", "custom_llm_provider": "parallel_ai", "number_of_queries": -1}"#;
    let answer = cost(text);
    let encoded = serde_json::to_string(&CostAnswer::Cost {
        input_cost: Decimal::new(5_i64, 3),
        output_cost: Decimal::ZERO,
    })
    .expect("the answer encodes");
    assert_eq!(answer, encoded);
}

#[test]
fn logging_state_names_the_search_call_after_its_provider() {
    let state = logging_state(&session(), "brave").expect("the logging state builds");
    assert_eq!(state.model, "brave/search");
    assert_eq!(state.call_type, "search");
}

#[test]
fn cost_reports_a_model_the_catalog_lacks() {
    let answer = cost(r#"{"model": "nonesuch/search", "number_of_queries": 1}"#);
    assert_eq!(
        answer,
        r#"{"outcome":"unmapped","message":"nonesuch/search isn't mapped yet"}"#
    );
}

#[test]
fn search_refuses_a_non_text_query_with_bad_request() {
    let call: SearchCall = serde_json::from_str(r#"{"query": 7, "search_provider": "brave"}"#)
        .expect("the search call parses");
    let outcome = get_runtime().block_on(session().search(call));
    match outcome.expect_err("the query is refused") {
        NativeFailure::Class { class, .. } => assert!(matches!(class, ErrorClass::BadRequest(_))),
        failure @ (NativeFailure::Convert(_) | NativeFailure::Session(_)) => {
            panic!("the failure is not a class: {failure:?}")
        }
    }
}

#[test]
fn search_fails_for_an_unknown_provider() {
    let call: SearchCall = serde_json::from_str(
        r#"{"query": "docs", "search_provider": "nonesuch", "api_base": "http://127.0.0.1:9"}"#,
    )
    .expect("the search call parses");
    let outcome = get_runtime().block_on(session().search(call));
    match outcome.expect_err("the provider is unknown") {
        NativeFailure::Class { class, message, .. } => {
            assert!(matches!(class, ErrorClass::BadRequest(_)));
            assert!(message.contains("Search is not supported for provider: nonesuch"));
        }
        failure @ (NativeFailure::Convert(_) | NativeFailure::Session(_)) => {
            panic!("the failure is not a class: {failure:?}")
        }
    }
}

#[test]
fn ocr_refuses_a_file_document_without_a_file() {
    let call: OcrCall =
        serde_json::from_str(r#"{"model": "mistral-ocr-latest", "document": {"type": "file"}}"#)
            .expect("the ocr call parses");
    let outcome = get_runtime().block_on(session().ocr(call, Vec::new()));
    match outcome.expect_err("the document is refused") {
        NativeFailure::Class { class, message, .. } => {
            assert!(matches!(class, ErrorClass::BadRequest(_)));
            assert!(message.contains("must include a 'file' field"));
        }
        failure @ (NativeFailure::Convert(_) | NativeFailure::Session(_)) => {
            panic!("the failure is not a class: {failure:?}")
        }
    }
}
