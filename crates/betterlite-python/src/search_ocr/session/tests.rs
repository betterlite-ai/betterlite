#![cfg(test)]

use pyo3_async_runtimes::tokio::get_runtime;
use rstest::rstest;
use rust_decimal::Decimal;

use super::SessionCore;
use crate::search_ocr::convert::{CostAnswer, CostCall, OcrCall, SearchCall};

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

#[test]
fn cost_reports_a_model_the_catalog_lacks() {
    let answer = cost(r#"{"model": "nonesuch/search", "number_of_queries": 1}"#);
    assert_eq!(
        answer,
        r#"{"outcome":"unmapped","message":"nonesuch/search isn't mapped yet"}"#
    );
}

#[test]
fn search_fails_for_an_unknown_provider() {
    let call: SearchCall = serde_json::from_str(
        r#"{"query": "docs", "search_provider": "nonesuch", "api_base": "http://127.0.0.1:9"}"#,
    )
    .expect("the search call parses");
    let outcome = get_runtime().block_on(session().search(call));
    let _failure = outcome.expect_err("the provider is unknown");
}

#[test]
fn ocr_refuses_a_file_document_without_a_file() {
    let call: OcrCall =
        serde_json::from_str(r#"{"model": "mistral-ocr-latest", "document": {"type": "file"}}"#)
            .expect("the ocr call parses");
    let outcome = get_runtime().block_on(session().ocr(call, &[]));
    let _failure = outcome.expect_err("the document is refused");
}
