use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_store_contracts::config::Params;
use rstest::rstest;
use serde_json::Number;

use super::support::{object, params};
use crate::s3_vectors_ingestion::{S3_VECTORS_STORE_ID_ERROR, S3VectorsInputError};
use crate::s3_vectors_search::error::S3VectorsSearchError;
use crate::s3_vectors_search::request::{DEFAULT_TOP_K, QueryInput, build_query_request};

const API_BASE: &str = "https://s3vectors.us-west-2.api.aws";

fn input<'request>(
    vector_store_id: &'request str,
    litellm_params: &'request Params,
    max_num_results: Option<i64>,
) -> QueryInput<'request> {
    QueryInput {
        vector_store_id,
        query_text: "what is a vector",
        query_vector: &[0.5, 1.0, -0.25],
        max_num_results,
        api_base: API_BASE,
        litellm_params,
    }
}

#[test]
fn query_request_has_the_exact_body_and_default_top_k() {
    let litellm_params = params("{}");
    let request = build_query_request(&input("docs:index-1", &litellm_params, None)).unwrap();
    assert_eq!(
        request.url,
        "https://s3vectors.us-west-2.api.aws/QueryVectors"
    );
    assert_eq!(request.query_text, "what is a vector");
    assert_eq!(DEFAULT_TOP_K, 5);
    let expected = object(
        r#"{"vectorBucketName": "docs", "indexName": "index-1",
            "queryVector": {"float32": [0.5, 1.0, -0.25]}, "topK": 5,
            "returnDistance": true, "returnMetadata": true}"#,
    );
    assert_eq!(request.body, expected);
    let keys: Vec<&str> = request.body.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "vectorBucketName",
            "indexName",
            "queryVector",
            "topK",
            "returnDistance",
            "returnMetadata"
        ]
    );
}

#[rstest]
#[case::explicit(Some(7), 7)]
#[case::zero_is_used_as_given(Some(0), 0)]
#[case::absent_takes_default(None, 5)]
fn top_k_comes_from_max_num_results(#[case] given: Option<i64>, #[case] expected: i64) {
    let litellm_params = params(r#"{"maxResults": 99}"#);
    let request = build_query_request(&input("docs:index-1", &litellm_params, given)).unwrap();
    assert_eq!(
        request.body.get("topK"),
        Some(&Json::Number(Number::from(expected)))
    );
}

#[rstest]
#[case::colon_splits_once("a:b:c", "{}", "a", "b:c")]
#[case::colon_ignores_fallback("a:b", r#"{"vector_bucket_name": "other"}"#, "a", "b")]
#[case::bare_index_uses_fallback("idx", r#"{"vector_bucket_name": "fallback"}"#, "fallback", "idx")]
fn the_target_comes_from_the_store_id(
    #[case] store_id: &str,
    #[case] litellm_params_json: &str,
    #[case] bucket: &str,
    #[case] index: &str,
) {
    let litellm_params = params(litellm_params_json);
    let request = build_query_request(&input(store_id, &litellm_params, None)).unwrap();
    assert_eq!(
        request.body.get("vectorBucketName"),
        Some(&Json::Text(bucket.to_owned()))
    );
    assert_eq!(
        request.body.get("indexName"),
        Some(&Json::Text(index.to_owned()))
    );
}

#[rstest]
#[case::no_bucket("idx", "{}")]
#[case::null_bucket("idx", r#"{"vector_bucket_name": null}"#)]
#[case::empty_bucket("idx", r#"{"vector_bucket_name": ""}"#)]
#[case::empty_store_id("", r#"{"vector_bucket_name": "fallback"}"#)]
#[case::empty_bucket_in_id(":idx", r#"{"vector_bucket_name": "fallback"}"#)]
#[case::empty_index_in_id("bucket:", "{}")]
fn a_missing_target_is_the_store_id_error(
    #[case] store_id: &str,
    #[case] litellm_params_json: &str,
) {
    let litellm_params = params(litellm_params_json);
    let error = build_query_request(&input(store_id, &litellm_params, None)).unwrap_err();
    assert!(matches!(
        error,
        S3VectorsSearchError::Target(S3VectorsInputError::StoreId)
    ));
    assert_eq!(error.to_string(), S3_VECTORS_STORE_ID_ERROR);
}

#[test]
fn a_non_text_bucket_is_refused_by_name() {
    let litellm_params = params(r#"{"vector_bucket_name": 12}"#);
    let error = build_query_request(&input("idx", &litellm_params, None)).unwrap_err();
    assert!(matches!(
        error,
        S3VectorsSearchError::ParamNotText {
            field: "vector_bucket_name"
        }
    ));
}

#[test]
fn a_bucket_in_the_store_id_never_reads_the_fallback() {
    let litellm_params = params(r#"{"vector_bucket_name": 12}"#);
    let request = build_query_request(&input("docs:idx", &litellm_params, None)).unwrap();
    assert_eq!(
        request.body.get("vectorBucketName"),
        Some(&Json::Text("docs".to_owned()))
    );
}

#[rstest]
#[case::nan(f64::NAN, 1)]
#[case::infinity(f64::INFINITY, 1)]
#[case::negative_infinity(f64::NEG_INFINITY, 1)]
fn a_vector_value_json_cannot_carry_is_refused(#[case] bad: f64, #[case] position: usize) {
    let litellm_params = params("{}");
    let vector = [0.5_f64, bad, 0.25_f64];
    let request = QueryInput {
        query_vector: &vector,
        ..input("docs:idx", &litellm_params, None)
    };
    let error = build_query_request(&request).unwrap_err();
    assert!(matches!(
        error,
        S3VectorsSearchError::NonFiniteVector { index } if index == position
    ));
}

#[test]
fn an_empty_vector_is_sent_as_an_empty_list() {
    let litellm_params = params("{}");
    let request = QueryInput {
        query_vector: &[],
        ..input("docs:idx", &litellm_params, None)
    };
    let body = build_query_request(&request).unwrap().body;
    assert_eq!(
        body.get("queryVector"),
        Some(&Json::Object(object(r#"{"float32": []}"#)))
    );
}
