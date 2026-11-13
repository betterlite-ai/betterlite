#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankDocument, RerankResponse, RerankResponseDocument,
};
use rstest::rstest;
use secrecy::SecretString;
use uuid::Uuid;

use super::ranking::NvidiaNimRankingConfig;
use super::reply::decode_nvidia_nim_rerank_reply;
use super::retrieval::NvidiaNimRerankConfig;
use super::{NvidiaNimRerankError, NvidiaNimRerankSelection, select_rerank_config};
use crate::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, Params, RerankLogging,
    RerankProviderResponse, ResponseTransform,
};

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

fn params(source: &str) -> Params {
    serde_json::from_str(source).unwrap()
}

fn documents(source: &str) -> Vec<RerankDocument> {
    serde_json::from_str(source).unwrap()
}

fn config() -> NvidiaNimRerankConfig {
    NvidiaNimRerankConfig {
        env_api_key: None,
        global_api_key: None,
        new_id: fixed_id,
    }
}

fn ranking() -> NvidiaNimRankingConfig {
    NvidiaNimRankingConfig {
        retrieval: config(),
        top_n: Mutex::new(None),
    }
}

#[derive(Clone, Copy)]
enum Family {
    Retrieval,
    Ranking,
}

fn mapped(
    family: Family,
    non_default: &mut Params,
    documents: &[RerankDocument],
    top_n: Option<i64>,
) -> Params {
    let request = MapCohereRerankParams {
        non_default_params: non_default,
        model: "meta/llama",
        drop_params: false,
        query: "q",
        documents,
        custom_llm_provider: Some("nvidia_nim"),
        top_n,
        rank_fields: None,
        return_documents: Some(true),
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        instruction: None,
    };
    match family {
        Family::Retrieval => config().map_cohere_rerank_params(request),
        Family::Ranking => ranking().map_cohere_rerank_params(request),
    }
    .unwrap()
}

fn request(
    config: &NvidiaNimRerankConfig,
    model: &str,
    optional: &Params,
) -> Result<Params, NvidiaNimRerankError> {
    config.transform_rerank_request(model, optional, &Headers::new(), None)
}

fn transformed(
    config: &dyn BaseRerankConfig<Error = NvidiaNimRerankError>,
    status: u16,
    body: &[u8],
    request_data: &Params,
    optional: &Params,
) -> Result<RerankResponse, NvidiaNimRerankError> {
    let provider_response = RerankProviderResponse {
        status_code: status,
        headers: Headers::from([(String::from("x-request"), String::from("1"))]),
        body: body.to_vec(),
    };
    let mut logging = RerankLogging::default();
    let litellm_params = Params::new();
    config.transform_rerank_response(
        RerankResponse::default(),
        ResponseTransform {
            model: "meta/llama",
            raw_response: &provider_response,
            logging: &mut logging,
            api_key: None,
            request_data,
            optional_params: optional,
            litellm_params: &litellm_params,
        },
    )
}

fn response(
    config: &NvidiaNimRerankConfig,
    status: u16,
    body: &[u8],
    request_data: &Params,
    optional: &Params,
) -> Result<RerankResponse, NvidiaNimRerankError> {
    transformed(config, status, body, request_data, optional)
}

fn url(config: &NvidiaNimRerankConfig, api_base: Option<&str>, model: &str) -> String {
    config.get_complete_url(api_base, model, None)
}

#[test]
fn select_rerank_config_reads_the_raw_ranking_prefix() {
    assert!(matches!(
        select_rerank_config("ranking/nvidia/llama"),
        NvidiaNimRerankSelection::Ranking(_)
    ));
    assert!(matches!(
        select_rerank_config("nvidia/llama"),
        NvidiaNimRerankSelection::Retrieval(_)
    ));
    assert!(matches!(
        select_rerank_config("nvidia_nim/ranking/nvidia/llama"),
        NvidiaNimRerankSelection::Retrieval(_)
    ));
}

#[test]
fn select_rerank_config_returns_a_config_of_the_matching_family() {
    let NvidiaNimRerankSelection::Ranking(selected) = select_rerank_config("ranking/x") else {
        panic!("a ranking model selects the ranking config");
    };
    assert_eq!(
        selected.get_complete_url(None, "ranking/x", None),
        "https://ai.api.nvidia.com/v1/ranking"
    );
    let NvidiaNimRerankSelection::Retrieval(selected) = select_rerank_config("x") else {
        panic!("any other model selects the retrieval config");
    };
    assert_eq!(
        selected.get_complete_url(None, "x", None),
        "https://ai.api.nvidia.com/v1/retrieval/x/reranking"
    );
}

#[rstest]
#[case::default_base(
    None,
    "meta/llama",
    "https://ai.api.nvidia.com/v1/retrieval/meta/llama/reranking"
)]
#[case::empty_base(
    Some(""),
    "meta/llama",
    "https://ai.api.nvidia.com/v1/retrieval/meta/llama/reranking"
)]
#[case::trailing_slash(
    Some("https://h/"),
    "meta/llama",
    "https://h/v1/retrieval/meta/llama/reranking"
)]
#[case::v1_trimmed(
    Some("https://h/v1"),
    "meta/llama",
    "https://h/v1/retrieval/meta/llama/reranking"
)]
#[case::v1_slash(
    Some("https://h/v1/"),
    "meta/llama",
    "https://h/v1/retrieval/meta/llama/reranking"
)]
#[case::full_url_kept(
    Some("https://h/v1/retrieval/meta/llama/reranking"),
    "meta/llama",
    "https://h/v1/retrieval/meta/llama/reranking"
)]
#[case::provider_prefix_stripped(
    Some("https://h"),
    "nvidia_nim/meta/llama-3_2",
    "https://h/v1/retrieval/meta/llama-3_2/reranking"
)]
#[case::ranking_prefix_kept(
    Some("https://h"),
    "ranking/meta/llama",
    "https://h/v1/retrieval/ranking/meta/llama/reranking"
)]
fn callback_retrieval_url(
    #[case] api_base: Option<&str>,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(url(&config(), api_base, model), expected);
}

#[rstest]
#[case::default_base(None, "https://ai.api.nvidia.com/v1/ranking")]
#[case::empty_base(Some(""), "https://ai.api.nvidia.com/v1/ranking")]
#[case::plain_base(Some("https://h"), "https://h/v1/ranking")]
#[case::v1_trimmed(Some("https://h/v1"), "https://h/v1/ranking")]
#[case::ranking_kept(Some("https://h/v1/ranking"), "https://h/v1/ranking")]
#[case::ranking_slash_kept(Some("https://h/v1/ranking/"), "https://h/v1/ranking")]
fn ranking_url(#[case] api_base: Option<&str>, #[case] expected: &str) {
    assert_eq!(
        ranking().get_complete_url(api_base, "ranking/meta/llama", None),
        expected
    );
}

#[test]
fn supported_rerank_params_are_the_three_cohere_names() {
    assert_eq!(
        config().supported_rerank_params("meta/llama"),
        ["query", "documents", "top_n"]
    );
    assert_eq!(
        ranking().supported_rerank_params("ranking/meta/llama"),
        ["query", "documents", "top_n"]
    );
}

#[test]
fn map_cohere_rerank_params_starts_from_the_query_and_documents() {
    let mut non_default = Params::new();
    let documents = documents(r#"["a", {"text": "b"}]"#);
    assert_eq!(
        mapped(Family::Retrieval, &mut non_default, &documents, None),
        params(r#"{"query": "q", "documents": ["a", {"text": "b"}]}"#)
    );
}

#[test]
fn map_cohere_rerank_params_maps_top_n_to_top_k() {
    let mut non_default = Params::new();
    let documents = documents(r#"["a"]"#);
    assert_eq!(
        mapped(Family::Retrieval, &mut non_default, &documents, Some(3)),
        params(r#"{"query": "q", "documents": ["a"], "top_k": 3}"#)
    );
    assert_eq!(
        mapped(Family::Retrieval, &mut non_default, &documents, Some(0)),
        params(r#"{"query": "q", "documents": ["a"], "top_k": 0}"#)
    );
}

#[test]
fn map_cohere_rerank_params_overlays_the_non_default_params() {
    let mut non_default = params(r#"{"query": "other", "documents": ["z"], "truncate": "END"}"#);
    let documents = documents(r#"["a"]"#);
    assert_eq!(
        mapped(Family::Retrieval, &mut non_default, &documents, Some(3)),
        params(r#"{"query": "other", "documents": ["z"], "truncate": "END", "top_k": 3}"#)
    );
}

#[test]
fn map_cohere_rerank_params_keeps_top_n_for_the_ranking_endpoint() {
    let mut non_default = params(r#"{"top_k": 9, "truncate": "END"}"#);
    let documents = documents(r#"["a"]"#);
    assert_eq!(
        mapped(Family::Ranking, &mut non_default, &documents, Some(4)),
        params(r#"{"query": "q", "documents": ["a"], "truncate": "END", "top_n": 4}"#)
    );
}

#[test]
fn map_cohere_rerank_params_lets_the_named_top_n_replace_the_request_one() {
    let mut non_default = params(r#"{"top_n": 2}"#);
    let documents = documents(r#"["a"]"#);
    assert_eq!(
        mapped(Family::Ranking, &mut non_default, &documents, Some(4)),
        params(r#"{"query": "q", "documents": ["a"], "top_n": 4}"#)
    );
}

#[test]
fn map_cohere_rerank_params_keeps_a_request_top_n_without_a_named_one() {
    let mut non_default = params(r#"{"top_n": 2}"#);
    let documents = documents(r#"["a"]"#);
    assert_eq!(
        mapped(Family::Ranking, &mut non_default, &documents, None),
        params(r#"{"query": "q", "documents": ["a"], "top_n": 2}"#)
    );
}

#[test]
fn validate_environment_fails_without_any_key() {
    let request = EnvironmentRequest {
        headers: &Headers::new(),
        model: "meta/llama",
        api_key: None,
        optional_params: None,
        litellm_params: None,
    };
    let error = config().validate_environment(request).unwrap_err();
    assert!(matches!(error, NvidiaNimRerankError::MissingApiKey));
}

#[test]
fn validate_environment_uses_the_call_key_before_the_environment_one() {
    let call_key = SecretString::from("call");
    let env_key = SecretString::from("env");
    let global_key = SecretString::from("global");
    let request = EnvironmentRequest {
        headers: &Headers::new(),
        model: "meta/llama",
        api_key: Some(&call_key),
        optional_params: None,
        litellm_params: None,
    };
    let headers = NvidiaNimRerankConfig {
        env_api_key: Some(env_key),
        global_api_key: Some(global_key),
        new_id: fixed_id,
    }
    .validate_environment(request)
    .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer call")
    );
}

#[test]
fn validate_environment_falls_back_to_the_environment_key() {
    let env_key = SecretString::from("env");
    let global_key = SecretString::from("global");
    let request = EnvironmentRequest {
        headers: &Headers::new(),
        model: "meta/llama",
        api_key: None,
        optional_params: None,
        litellm_params: None,
    };
    let headers = NvidiaNimRerankConfig {
        env_api_key: Some(env_key),
        global_api_key: Some(global_key),
        new_id: fixed_id,
    }
    .validate_environment(request)
    .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer env")
    );
}

#[test]
fn validate_environment_falls_back_to_the_global_key() {
    let global_key = SecretString::from("global");
    let request = EnvironmentRequest {
        headers: &Headers::new(),
        model: "meta/llama",
        api_key: None,
        optional_params: None,
        litellm_params: None,
    };
    let headers = NvidiaNimRerankConfig {
        env_api_key: None,
        global_api_key: Some(global_key),
        new_id: fixed_id,
    }
    .validate_environment(request)
    .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer global")
    );
}

#[test]
fn validate_environment_uses_an_empty_call_key_as_given() {
    let call_key = SecretString::from("");
    let env_key = SecretString::from("env");
    let request = EnvironmentRequest {
        headers: &Headers::new(),
        model: "meta/llama",
        api_key: Some(&call_key),
        optional_params: None,
        litellm_params: None,
    };
    let headers = NvidiaNimRerankConfig {
        env_api_key: Some(env_key),
        global_api_key: None,
        new_id: fixed_id,
    }
    .validate_environment(request)
    .unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer ")
    );
}

#[test]
fn validate_environment_adds_the_default_headers_and_lets_the_caller_override_them() {
    let call_key = SecretString::from("call");
    let caller_headers = Headers::from([
        (String::from("Authorization"), String::from("caller")),
        (String::from("accept"), String::from("text/plain")),
        (String::from("x-extra"), String::from("kept")),
    ]);
    let request = EnvironmentRequest {
        headers: &caller_headers,
        model: "meta/llama",
        api_key: Some(&call_key),
        optional_params: None,
        litellm_params: None,
    };
    let headers = config().validate_environment(request).unwrap();
    assert_eq!(
        headers,
        Headers::from([
            (String::from("Authorization"), String::from("caller")),
            (String::from("accept"), String::from("text/plain")),
            (
                String::from("content-type"),
                String::from("application/json")
            ),
            (String::from("x-extra"), String::from("kept")),
        ])
    );
}

#[test]
fn transform_rerank_request_needs_a_query_and_documents() {
    assert!(matches!(
        request(&config(), "meta/llama", &Params::new()).unwrap_err(),
        NvidiaNimRerankError::MissingQuery
    ));
    assert!(matches!(
        request(&config(), "meta/llama", &params(r#"{"query": "q"}"#)).unwrap_err(),
        NvidiaNimRerankError::MissingDocuments
    ));
    assert!(matches!(
        request(
            &config(),
            "meta/llama",
            &params(r#"{"query": "q", "documents": "a"}"#)
        )
        .unwrap_err(),
        NvidiaNimRerankError::DocumentsNotAList
    ));
}

#[test]
fn transform_rerank_request_builds_the_nim_body() {
    let optional = params(
        r#"{"query": "q", "documents": ["a", {"text": "b", "image": "c"}, {"image": "d"}, {"title": "e"}], "top_k": 2, "truncate": "END"}"#,
    );
    let body = request(&config(), "nvidia_nim/meta/llama-3_2", &optional).unwrap();
    assert_eq!(
        body,
        params(
            r#"{"model": "meta.llama-3.2", "query": {"text": "q"}, "passages": [{"text": "a"}, {"text": "b"}, {"text": "{\"image\":\"d\"}"}, {"text": "{\"title\":\"e\"}"}], "top_k": 2, "truncate": "END"}"#
        )
    );
}

#[test]
fn transform_rerank_request_drops_truncate_values_other_than_none_and_end() {
    let optional = params(r#"{"query": "q", "documents": [], "truncate": "START"}"#);
    let body = request(&config(), "m", &optional).unwrap();
    assert!(!body.contains_key("truncate"));
}

#[test]
fn transform_rerank_request_keeps_truncate_none() {
    let optional = params(r#"{"query": "q", "documents": [], "truncate": "NONE"}"#);
    let body = request(&config(), "m", &optional).unwrap();
    assert_eq!(
        body.get("truncate"),
        Some(&Json::Text(String::from("NONE")))
    );
}

#[test]
fn transform_rerank_request_leaves_out_a_null_top_k() {
    let optional = params(r#"{"query": "q", "documents": [], "top_k": null}"#);
    let body = request(&config(), "m", &optional).unwrap();
    assert!(!body.contains_key("top_k"));
}

#[test]
fn transform_rerank_request_rejects_a_document_that_is_neither_string_nor_object() {
    let optional = params(r#"{"query": "q", "documents": ["a", 5]}"#);
    assert!(matches!(
        request(&config(), "m", &optional).unwrap_err(),
        NvidiaNimRerankError::UnsupportedDocument { position: 1 }
    ));
}

#[test]
fn transform_rerank_request_keeps_the_image_only_for_the_ranking_config() {
    let optional = params(r#"{"query": "q", "documents": [{"text": "a", "image": "b"}]}"#);
    assert_eq!(
        request(&config(), "m", &optional).unwrap(),
        params(r#"{"model": "m", "query": {"text": "q"}, "passages": [{"text": "a"}]}"#)
    );
    let ranking_body = ranking()
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    assert_eq!(
        ranking_body,
        params(
            r#"{"model": "m", "query": {"text": "q"}, "passages": [{"text": "a", "image": "b"}]}"#
        )
    );
}

#[test]
fn transform_rerank_request_strips_the_ranking_prefix_from_the_body_model() {
    let optional = params(r#"{"query": "q", "documents": []}"#);
    let body = ranking()
        .transform_rerank_request(
            "nvidia_nim/ranking/meta/llama-3_2",
            &optional,
            &Headers::new(),
            None,
        )
        .unwrap();
    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from("meta.llama-3.2")))
    );
}

#[test]
fn transform_rerank_request_removes_top_n_and_top_k_from_the_ranking_body() {
    let optional = params(r#"{"query": "q", "documents": [], "top_n": 3, "top_k": 9}"#);
    let body = ranking()
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    assert!(!body.contains_key("top_n"));
    assert!(!body.contains_key("top_k"));
}

#[rstest]
#[case::zero(r#"{"top_n": 0}"#)]
#[case::negative(r#"{"top_n": -2}"#)]
#[case::text(r#"{"top_n": "3"}"#)]
#[case::boolean(r#"{"top_n": true}"#)]
#[case::fractional(r#"{"top_n": 1.5}"#)]
fn transform_rerank_request_rejects_a_top_n_that_is_not_a_positive_integer(#[case] source: &str) {
    let optional = params_with(source);
    let error = ranking()
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap_err();
    assert!(matches!(
        error,
        NvidiaNimRerankError::TopNNotPositive { .. }
    ));
}

fn params_with(source: &str) -> Params {
    let mut optional = params(source);
    optional.insert("query".to_owned(), Json::Text(String::from("q")));
    optional.insert("documents".to_owned(), Json::List(Vec::new()));
    optional
}

#[test]
fn transform_rerank_request_keeps_a_top_n_of_one() {
    let config = ranking();
    let optional = params_with(r#"{"top_n": 1}"#);
    config
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    assert_eq!(*config.top_n.lock().unwrap(), Some(1));
}

#[test]
fn transform_rerank_request_skips_a_null_or_absent_top_n() {
    let config = ranking();
    let optional = params_with(r#"{"top_n": null}"#);
    config
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    assert_eq!(*config.top_n.lock().unwrap(), None);
    let optional = params_with("{}");
    config
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    assert_eq!(*config.top_n.lock().unwrap(), None);
}

#[test]
fn transform_rerank_response_reports_a_body_that_is_not_json() {
    let body = b"not json";
    let error = response(&config(), 502, body, &Params::new(), &Params::new()).unwrap_err();
    let NvidiaNimRerankError::Upstream(exception) = error else {
        panic!("a body that is not json is an upstream exception");
    };
    assert_eq!(exception.status_code, 502);
    assert_eq!(exception.message, "not json");
    assert_eq!(
        exception.headers.get("x-request"),
        Some(&Json::Text(String::from("1")))
    );
}

#[rstest]
#[case::array("[]")]
#[case::missing_index(r#"{"rankings": [{"logit": 0.5}]}"#)]
#[case::missing_logit(r#"{"rankings": [{"index": 0}]}"#)]
#[case::text_index(r#"{"rankings": [{"index": "0", "logit": 0.5}]}"#)]
#[case::text_logit(r#"{"rankings": [{"index": 0, "logit": "0.5"}]}"#)]
#[case::null_rankings(r#"{"rankings": null}"#)]
fn transform_rerank_response_reports_a_body_of_the_wrong_shape(#[case] body: &str) {
    let error = response(
        &config(),
        200,
        body.as_bytes(),
        &Params::new(),
        &Params::new(),
    )
    .unwrap_err();
    assert!(matches!(error, NvidiaNimRerankError::ReplyShape { .. }));
}

#[test]
fn transform_rerank_response_builds_the_results_and_metadata() {
    let body = br#"{"id": "reply", "rankings": [{"index": 1, "logit": 0.25}, {"index": 0, "logit": 0.5}], "usage": {"total_tokens": 4}}"#;
    let request_data = params(r#"{"passages": [{"text": "first"}, {"text": "second"}]}"#);
    let response = response(&config(), 200, body, &request_data, &Params::new()).unwrap();
    assert_eq!(response.id.as_deref(), Some("reply"));
    let results = response.results.unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].index, 1);
    assert_eq!(
        results[0].document,
        Some(RerankResponseDocument {
            text: String::from("second")
        })
    );
    assert_eq!(results[1].index, 0);
    assert_eq!(
        results[1].document,
        Some(RerankResponseDocument {
            text: String::from("first")
        })
    );
    let billed = response.meta.unwrap().billed_units.unwrap().unwrap();
    assert_eq!(billed.total_tokens, Some(Some(4)));
}

#[test]
fn transform_rerank_response_counts_the_results_when_the_usage_is_zero() {
    let body = br#"{"rankings": [{"index": 0, "logit": 0.5}, {"index": 1, "logit": 0.25}], "usage": {"total_tokens": 0}}"#;
    let request_data = params(r#"{"passages": []}"#);
    let response = response(&config(), 200, body, &request_data, &Params::new()).unwrap();
    let billed = response.meta.unwrap().billed_units.unwrap().unwrap();
    assert_eq!(billed.total_tokens, Some(Some(2)));
}

#[test]
fn transform_rerank_response_counts_the_results_without_a_usage_field() {
    let body = br#"{"rankings": [{"index": 0, "logit": 0.5}]}"#;
    let response = response(
        &config(),
        200,
        body,
        &params(r#"{"passages": []}"#),
        &Params::new(),
    )
    .unwrap();
    let billed = response.meta.unwrap().billed_units.unwrap().unwrap();
    assert_eq!(billed.total_tokens, Some(Some(1)));
}

#[test]
fn transform_rerank_response_generates_an_id_when_the_body_has_none() {
    let body = br#"{"rankings": []}"#;
    let response = response(&config(), 200, body, &Params::new(), &Params::new()).unwrap();
    assert_eq!(
        response.id.as_deref(),
        Some("00000000-0000-0000-0000-000000000007")
    );
}

#[test]
fn transform_rerank_response_keeps_an_empty_id_as_given() {
    let body = br#"{"id": "", "rankings": []}"#;
    let response = response(&config(), 200, body, &Params::new(), &Params::new()).unwrap();
    assert_eq!(response.id.as_deref(), Some(""));
}

#[test]
fn transform_rerank_response_reads_a_negative_index_from_the_end() {
    let body = br#"{"rankings": [{"index": -1, "logit": 0.5}]}"#;
    let request_data = params(r#"{"passages": [{"text": "first"}, {"text": "last"}]}"#);
    let response = response(&config(), 200, body, &request_data, &Params::new()).unwrap();
    assert_eq!(
        response.results.unwrap()[0].document,
        Some(RerankResponseDocument {
            text: String::from("last")
        })
    );
}

#[test]
fn transform_rerank_response_leaves_out_a_document_past_the_passages() {
    let body = br#"{"rankings": [{"index": 2, "logit": 0.5}]}"#;
    let request_data = params(r#"{"passages": [{"text": "first"}, {"text": "last"}]}"#);
    let response = response(&config(), 200, body, &request_data, &Params::new()).unwrap();
    assert_eq!(response.results.unwrap()[0].document, None);
}

#[test]
fn transform_rerank_response_reports_an_index_below_the_passages() {
    let body = br#"{"rankings": [{"index": -3, "logit": 0.5}]}"#;
    let request_data = params(r#"{"passages": [{"text": "first"}, {"text": "last"}]}"#);
    let error = response(&config(), 200, body, &request_data, &Params::new()).unwrap_err();
    assert!(matches!(
        error,
        NvidiaNimRerankError::RankingIndex {
            index: -3,
            passages: 2
        }
    ));
}

#[test]
fn transform_rerank_response_leaves_out_a_document_for_an_image_only_passage() {
    let body = br#"{"rankings": [{"index": 0, "logit": 0.5}, {"index": 1, "logit": 0.25}]}"#;
    let request_data = params(r#"{"passages": [{"image": "a"}, {"text": "b"}]}"#);
    let response = response(&config(), 200, body, &request_data, &Params::new()).unwrap();
    let results = response.results.unwrap();
    assert_eq!(results[0].document, None);
    assert_eq!(
        results[1].document,
        Some(RerankResponseDocument {
            text: String::from("b")
        })
    );
}

#[test]
fn transform_rerank_response_reports_a_document_text_that_is_not_a_string() {
    let body = br#"{"rankings": [{"index": 0, "logit": 0.5}]}"#;
    let request_data = params(r#"{"passages": [{"text": 5}]}"#);
    let error = response(&config(), 200, body, &request_data, &Params::new()).unwrap_err();
    assert!(matches!(error, NvidiaNimRerankError::DocumentTextNotString));
}

fn ranking_results(config: &NvidiaNimRankingConfig, body: &[u8], optional: &Params) -> Vec<i64> {
    let response = transformed(config, 200, body, &Params::new(), optional).unwrap();
    response
        .results
        .unwrap()
        .iter()
        .map(|result| result.index)
        .collect()
}

#[test]
fn ranking_response_sorts_and_keeps_the_first_top_n() {
    let config = ranking();
    let optional = params_with(r#"{"top_n": 2}"#);
    config
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    let body = br#"{"rankings": [{"index": 0, "logit": 0.1}, {"index": 1, "logit": 0.9}, {"index": 2, "logit": 0.5}]}"#;
    assert_eq!(ranking_results(&config, body, &Params::new()), [1, 2]);
}

#[test]
fn ranking_response_keeps_the_order_when_no_sort_is_needed() {
    let config = ranking();
    let optional = params_with(r#"{"top_n": 3}"#);
    config
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    let body = br#"{"rankings": [{"index": 0, "logit": 0.1}, {"index": 1, "logit": 0.9}, {"index": 2, "logit": 0.5}]}"#;
    assert_eq!(ranking_results(&config, body, &Params::new()), [0, 1, 2]);
}

#[test]
fn ranking_response_sorts_stably_on_ties() {
    let config = ranking();
    let optional = params_with(r#"{"top_n": 2}"#);
    config
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    let body = br#"{"rankings": [{"index": 0, "logit": 0.5}, {"index": 1, "logit": 0.5}, {"index": 2, "logit": 0.5}]}"#;
    assert_eq!(ranking_results(&config, body, &Params::new()), [0, 1]);
}

#[test]
fn ranking_response_uses_the_request_top_n_when_it_is_truthy() {
    let config = ranking();
    let optional = params_with(r#"{"top_n": 3}"#);
    config
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    let body = br#"{"rankings": [{"index": 0, "logit": 0.1}, {"index": 1, "logit": 0.9}, {"index": 2, "logit": 0.5}]}"#;
    let request_optional = params(r#"{"top_n": 2}"#);
    assert_eq!(ranking_results(&config, body, &request_optional), [1, 2]);
}

#[test]
fn ranking_response_falls_back_to_the_kept_top_n_for_a_zero_request_top_n() {
    let config = ranking();
    let optional = params_with(r#"{"top_n": 2}"#);
    config
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    let body = br#"{"rankings": [{"index": 0, "logit": 0.1}, {"index": 1, "logit": 0.9}, {"index": 2, "logit": 0.5}]}"#;
    let request_optional = params(r#"{"top_n": 0}"#);
    assert_eq!(ranking_results(&config, body, &request_optional), [1, 2]);
}

#[test]
fn ranking_response_leaves_every_result_without_a_top_n() {
    let config = ranking();
    let body = br#"{"rankings": [{"index": 0, "logit": 0.1}, {"index": 1, "logit": 0.9}]}"#;
    assert_eq!(ranking_results(&config, body, &Params::new()), [0, 1]);
}

#[test]
fn ranking_response_reads_the_top_n_a_fresh_request_carries() {
    let config = ranking();
    let optional = params_with(r#"{"top_n": 1}"#);
    config
        .transform_rerank_request("ranking/m", &optional, &Headers::new(), None)
        .unwrap();
    let body = br#"{"rankings": [{"index": 0, "logit": 0.1}, {"index": 1, "logit": 0.9}]}"#;
    assert_eq!(ranking_results(&config, body, &Params::new()), [1]);
}

#[rstest]
fn replays_the_nvidia_nim_rerank_reply_corpus(
    #[files("fuzz/corpus/nvidia_nim_rerank_reply/*")] path: PathBuf,
) {
    let body = fs::read(path).unwrap();
    let _decoded = decode_nvidia_nim_rerank_reply(&body);
}
