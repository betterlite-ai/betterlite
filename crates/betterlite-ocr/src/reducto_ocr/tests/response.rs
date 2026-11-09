use rstest::rstest;
use serde_json::Number;

use crate::json::Json;
use crate::ocr_config::OcrProviderConfig as _;
use crate::ocr_error::OcrError;
use crate::ocr_http::tests::support::object;
use crate::request_format::RequestFormat;
use crate::response::OCRPage;

use super::{decode, text, v3};

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn decimal(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("the test number is finite"))
}

fn block_values(page: &OCRPage) -> Vec<Json> {
    match page.extra.get("blocks") {
        Some(Json::List(blocks)) => blocks.clone(),
        Some(_) | None => Vec::new(),
    }
}

fn page_index_markdown(page: &OCRPage) -> Json {
    Json::Object(object(&[
        ("index", number(page.index)),
        ("markdown", text(&page.markdown)),
    ]))
}

#[rstest]
#[case::null_result(r#"{"result":null,"chunks":[{"content":"ignored"}]}"#, Vec::new())]
#[case::absent_result(
    r#"{"chunks":[{"content":"text"}]}"#,
    vec![Json::Object(object(&[("index", number(0)), ("markdown", text("text"))]))]
)]
#[case::object_result(
    r#"{"result":{"chunks":[{"content":"kept"}]}}"#,
    vec![Json::Object(object(&[("index", number(0)), ("markdown", text("kept"))]))]
)]
fn the_result_shapes_choose_their_own_pages(#[case] raw: &str, #[case] expected: Vec<Json>) {
    let response = v3()
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect("the response normalizes");
    let pages: Vec<Json> = response.pages.iter().map(page_index_markdown).collect();
    assert_eq!(pages, expected);
}

#[test]
fn pages_group_blocks_by_page_and_usage_accepts_lax_numbers() {
    let raw = br#"{"usage":{"num_pages":"2","credits":true},"result":{"type":"full","chunks":[{"blocks":[{"type":"Table","content":"B","bbox":{"left":0.1,"page":2,"original_page":4},"confidence":"high","image_url":null}]},{"blocks":[{"content":"A","bbox":{"page":1}},{"content":"C","bbox":{"page":1}}]}]}}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect("the response normalizes");
    assert_eq!(response.model, "parse-v3");
    assert_eq!(response.pages.len(), 2);
    assert_eq!(response.pages[0].index, 0);
    assert_eq!(response.pages[0].markdown, "A\n\nC");
    assert_eq!(response.pages[1].index, 1);
    assert_eq!(response.pages[1].markdown, "B");
    assert_eq!(
        block_values(&response.pages[1]),
        vec![Json::Object(object(&[
            ("type", text("Table")),
            ("content", text("B")),
            (
                "bbox",
                Json::Object(object(&[
                    ("left", decimal(0.1)),
                    ("page", number(2)),
                    ("original_page", number(4)),
                ]))
            ),
            ("confidence", text("high")),
            ("image_url", Json::Null),
        ]))]
    );
    let usage = response.usage_info.expect("usage is reported");
    assert_eq!(usage.pages_processed, Some(2));
    assert_eq!(usage.credits, Some(1.0_f64));
}

#[test]
fn a_response_without_a_page_number_joins_the_chunk_content() {
    let raw = br#"{"chunks":[{"content":"one"},{"content":""},{"content":"two"}]}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect("the response normalizes");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, 0);
    assert_eq!(response.pages[0].markdown, "one\n\ntwo");
    assert!(response.pages[0].extra.is_empty());
    assert!(response.usage_info.is_some());
}

#[test]
fn a_block_content_that_is_not_text_fails_with_a_response_field() {
    let raw = br#"{"result":{"chunks":[{"blocks":[{"content":5,"bbox":{"page":1}}]}]}}"#;
    let error = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect_err("a non-text block content fails");
    assert!(
        matches!(&error, OcrError::ResponseField { path } if path == "result.chunks.blocks.content"),
        "{error:?}"
    );
}

#[test]
fn a_bad_block_page_number_leaves_the_block_out() {
    let raw = br#"{"result":{"chunks":[{"blocks":[{"content":"ignored","bbox":{"page":"invalid"}},{"content":"kept","bbox":{"page":2.5},"extra":null}]}]}}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect("a bad page number is best effort");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, 1);
    assert_eq!(response.pages[0].markdown, "kept");
    assert_eq!(
        block_values(&response.pages[0]),
        vec![Json::Object(object(&[
            ("content", text("kept")),
            ("bbox", Json::Object(object(&[("page", decimal(2.5))])),),
            ("extra", Json::Null),
        ]))]
    );
}

#[test]
fn a_block_page_number_at_two_pow_63_saturates_at_i64_max() {
    let raw =
        br#"{"chunks":[{"blocks":[{"content":"kept","bbox":{"page":9223372036854775808.0}}]}]}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect("the block page saturates");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, i64::MAX - 1);
    assert_eq!(response.pages[0].markdown, "kept");
}

#[rstest]
#[case::pages_integral_float(r#"{"usage":{"num_pages":2.0}}"#, Some(2), None)]
#[case::pages_numeric_string_with_fraction(r#"{"usage":{"num_pages":"2.0"}}"#, Some(2), None)]
#[case::pages_numeric_string_with_underscore(
    r#"{"usage":{"num_pages":"1_000"}}"#,
    Some(1000),
    None
)]
#[case::pages_boolean(r#"{"usage":{"num_pages":true}}"#, Some(1), None)]
#[case::credits_numeric_string(r#"{"usage":{"credits":"3"}}"#, None, Some(3.0_f64))]
#[case::credits_boolean(r#"{"usage":{"credits":true}}"#, None, Some(1.0_f64))]
fn usage_numbers_accept_the_forms_litellm_accepts(
    #[case] raw: &str,
    #[case] pages: Option<i64>,
    #[case] credits: Option<f64>,
) {
    let usage = v3()
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect("the usage parses")
        .usage_info
        .expect("usage is reported");
    assert_eq!(usage.pages_processed, pages);
    assert_eq!(usage.credits, credits);
}

#[rstest]
#[case::trimmed_string(
    r#"{"chunks":[{"blocks":[{"content":"a","bbox":{"page":" 3 "}}]}]}"#,
    2
)]
#[case::boolean(r#"{"chunks":[{"blocks":[{"content":"a","bbox":{"page":true}}]}]}"#, 0)]
#[case::zero(r#"{"chunks":[{"blocks":[{"content":"a","bbox":{"page":0}}]}]}"#, 0)]
#[case::negative(r#"{"chunks":[{"blocks":[{"content":"a","bbox":{"page":-1}}]}]}"#, 0)]
fn block_page_numbers_coerce_and_clamp(#[case] raw: &str, #[case] expected: i64) {
    let response = v3()
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect("the page parses");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, expected);
    assert_eq!(response.pages[0].markdown, "a");
}

#[rstest]
#[case::fractional_pages(r#"{"usage":{"num_pages":1.5}}"#)]
#[case::array_pages(r#"{"usage":{"num_pages":[]}}"#)]
#[case::object_credits(r#"{"usage":{"credits":{}}}"#)]
#[case::pages_over_i64(r#"{"usage":{"num_pages":9223372036854775808}}"#)]
#[case::infinite_credits(r#"{"usage":{"credits":"inf"}}"#)]
#[case::pages_exactly_two_pow_63(r#"{"usage":{"num_pages":9223372036854775808.0}}"#)]
#[case::text_credits(r#"{"usage":{"credits":"much"}}"#)]
#[case::fractional_numeric_string(r#"{"usage":{"num_pages":"2.5"}}"#)]
#[case::empty_fraction(r#"{"usage":{"num_pages":"2."}}"#)]
#[case::trailing_underscore(r#"{"usage":{"num_pages":"2_"}}"#)]
#[case::double_underscore(r#"{"usage":{"num_pages":"1__0"}}"#)]
#[case::signed_leading_underscore(r#"{"usage":{"num_pages":"+_2"}}"#)]
fn a_usage_value_of_the_wrong_type_fails_with_a_response_field(#[case] raw: &str) {
    let error = v3()
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect_err("a mistyped usage value fails");
    assert!(matches!(error, OcrError::ResponseField { .. }), "{error:?}");
}

#[test]
fn the_native_request_format_keeps_the_provider_json() {
    let raw = br#"{"chunks":[{"content":"text"}],"job_id":"job_1"}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Native)
        .expect("the response normalizes");
    assert_eq!(response.provider_native_response, Some(decode(raw)));
}
