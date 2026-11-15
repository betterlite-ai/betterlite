use betterlite_openai::api::vector_store_records::records::basic::VectorStoreOpenAiParam;
use betterlite_provider::api::vector_store_contracts::config::Headers;
use rstest::rstest;

use super::support::params;
use crate::s3_vectors_search::config::{
    RegionEnvironment, SUPPORTED_OPENAI_PARAMS, auth_credentials, complete_url, endpoints_by_type,
    map_openai_params, query_embedding_model, validate_environment,
};
use crate::s3_vectors_search::create::{CreateStep, transform_create_vector_store};
use crate::s3_vectors_search::error::S3VectorsSearchError;

#[test]
fn static_surface_matches_litellm() {
    let credentials = auth_credentials();
    assert!(credentials.headers.is_none() && credentials.query_params.is_none());
    let endpoints = endpoints_by_type();
    assert_eq!(
        endpoints.read,
        Some(vec![("POST".to_owned(), "/QueryVectors".to_owned())])
    );
    assert_eq!(endpoints.write, Some(Vec::new()));
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [VectorStoreOpenAiParam::MaxNumResults]
    );
}

#[test]
fn map_openai_params_stores_max_results_only() {
    let mapped = map_openai_params(
        &params(r#"{"max_num_results": 3, "filters": {"a": 1}}"#),
        params(r#"{"kept": true}"#),
    );
    assert_eq!(mapped, params(r#"{"kept": true, "maxResults": 3}"#));
    let untouched = map_openai_params(&params(r#"{"filters": {}}"#), params("{}"));
    assert!(untouched.is_empty());
}

#[rstest]
#[case::absent(&[], "application/json")]
#[case::given(&[("Content-Type", "text/plain")], "text/plain")]
fn validate_environment_sets_content_type_only_when_absent(
    #[case] given: &[(&str, &str)],
    #[case] expected: &str,
) {
    let headers: Headers = given
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let validated = validate_environment(headers);
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some(expected)
    );
}

#[test]
fn validate_environment_compares_header_names_exactly() {
    let headers = Headers::from([("content-type".to_owned(), "text/plain".to_owned())]);
    let validated = validate_environment(headers);
    assert_eq!(validated.len(), 2);
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[rstest]
#[case::explicit(r#"{"aws_region_name": "eu-west-1"}"#, None, None, "eu-west-1")]
#[case::explicit_beats_environment(
    r#"{"aws_region_name": "eu-west-1"}"#,
    Some("ap-south-1"),
    Some("us-east-2"),
    "eu-west-1"
)]
#[case::null_takes_environment(
    r#"{"aws_region_name": null}"#,
    Some("ap-south-1"),
    Some("us-east-2"),
    "us-east-2"
)]
#[case::region_name_environment("{}", Some("ap-south-1"), None, "ap-south-1")]
#[case::default("{}", None, None, "us-west-2")]
fn complete_url_uses_the_region_alone(
    #[case] litellm_params: &str,
    #[case] region_name: Option<&str>,
    #[case] region: Option<&str>,
    #[case] expected_region: &str,
) {
    let url = complete_url(
        &params(litellm_params),
        RegionEnvironment {
            region_name,
            region,
        },
    )
    .unwrap();
    assert_eq!(url, format!("https://s3vectors.{expected_region}.api.aws"));
}

#[test]
fn complete_url_rejects_a_bad_region_and_a_non_text_region() {
    let bad = complete_url(
        &params(r#"{"aws_region_name": "not a region"}"#),
        RegionEnvironment::default(),
    );
    assert!(matches!(bad, Err(S3VectorsSearchError::Region(_))));
    let number = complete_url(
        &params(r#"{"aws_region_name": 5}"#),
        RegionEnvironment::default(),
    );
    assert!(matches!(
        number,
        Err(S3VectorsSearchError::ParamNotText {
            field: "aws_region_name"
        })
    ));
}

#[rstest]
#[case::default("{}", "text-embedding-3-small")]
#[case::litellm_model_first(
    r#"{"litellm_embedding_model": "voyage/a", "embedding_model": "cohere/b"}"#,
    "voyage/a"
)]
#[case::embedding_model_second(r#"{"embedding_model": "cohere/b"}"#, "cohere/b")]
#[case::empty_first_falls_through(
    r#"{"litellm_embedding_model": "", "embedding_model": "cohere/b"}"#,
    "cohere/b"
)]
#[case::null_first_falls_through(
    r#"{"litellm_embedding_model": null, "embedding_model": "cohere/b"}"#,
    "cohere/b"
)]
#[case::both_empty(
    r#"{"litellm_embedding_model": "", "embedding_model": ""}"#,
    "text-embedding-3-small"
)]
fn query_embedding_model_picks_the_first_non_empty_name(
    #[case] litellm_params: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        query_embedding_model(&params(litellm_params)).unwrap(),
        expected
    );
}

#[test]
fn query_embedding_model_rejects_a_non_text_model() {
    let error = query_embedding_model(&params(r#"{"embedding_model": ["a"]}"#)).unwrap_err();
    assert!(matches!(
        error,
        S3VectorsSearchError::ParamNotText {
            field: "embedding_model"
        }
    ));
}

#[rstest]
#[case::request(CreateStep::Request)]
#[case::response(CreateStep::Response)]
fn creating_a_store_is_not_supported(#[case] step: CreateStep) {
    assert!(matches!(
        transform_create_vector_store(step),
        Err(S3VectorsSearchError::CreateNotSupported)
    ));
}
