#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::str;
use std::sync::Arc;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::api::rerank_contracts::records::{
    RerankResponse, RerankResponseDocument, RerankResponseMeta, RerankResponseResult, RerankTokens,
};
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use super::WatsonxRerankConfig;
use super::error::WatsonxRerankError;
use super::params::map_cohere_rerank_params;
use super::request::transform_rerank_request;
use crate::api::rerank_contracts::{
    Headers, MapCohereRerankParams, Params, RerankProviderResponse,
};

struct NoTransport;

impl WatsonxTransport for NoTransport {
    fn send_buffered(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Err(TransportError::Closed))
    }
}

fn environment(values: &[(&str, &str)]) -> WatsonxEnvironment {
    let find = |name: &str| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    };
    let secret = |name: &str| find(name).map(|value| SecretString::from(value.to_owned()));
    let text = |name: &str| find(name).map(str::to_owned);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

fn auth(environment: WatsonxEnvironment) -> WatsonxAuth<NoTransport> {
    WatsonxAuth::new(environment, NoTransport, Arc::new(|| 0.0_f64)).unwrap()
}

fn fixed_id() -> Uuid {
    Uuid::nil()
}

fn config(auth: &WatsonxAuth<NoTransport>) -> WatsonxRerankConfig<'_, NoTransport> {
    WatsonxRerankConfig {
        auth,
        new_id: fixed_id,
    }
}

fn params(wire: &str) -> Params {
    serde_json::from_str(wire).unwrap()
}

fn headers(wire: &[(&str, &str)]) -> Headers {
    wire.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn map(non_default: &mut Params) -> Result<Params, WatsonxRerankError> {
    map_cohere_rerank_params(&MapCohereRerankParams {
        non_default_params: non_default,
        model: "model",
        drop_params: false,
        query: "query",
        documents: &[],
        custom_llm_provider: None,
        top_n: None,
        rank_fields: None,
        return_documents: None,
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        instruction: None,
    })
}

fn mapped(values: &str) -> Params {
    map(&mut params(values)).unwrap()
}

fn response(status_code: u16, body: &[u8]) -> RerankProviderResponse {
    RerankProviderResponse {
        status_code,
        headers: Headers::new(),
        body: body.to_vec(),
    }
}

fn score(value: f64) -> Score {
    Score::try_from(Number::from_f64(value).unwrap()).unwrap()
}

fn meta(tokens: RerankTokens) -> RerankResponseMeta {
    RerankResponseMeta {
        api_version: None,
        billed_units: None,
        tokens: Some(Some(tokens)),
    }
}

#[test]
fn params_map_copies_every_mapped_key() {
    assert_eq!(
        mapped(
            r#"{"query":"q","documents":["a",{"text":"b","x":1}],"top_n":2,"return_documents":true,"max_tokens_per_doc":8,"project_id":"p","space_id":"s","instruction":"i","max_chunks_per_doc":4}"#
        ),
        params(
            r#"{"query":"q","inputs":[{"text":"a"},{"text":"b","x":1}],"parameters":{"return_options":{"top_n":2,"inputs":true},"truncate_input_tokens":8},"project_id":"p","space_id":"s"}"#
        )
    );
}

#[test]
fn params_map_ignores_null_and_unknown_keys() {
    assert_eq!(
        mapped(
            r#"{"query":null,"documents":null,"top_n":null,"return_documents":null,"max_tokens_per_doc":null,"project_id":null,"space_id":null,"other":1}"#
        ),
        Params::new()
    );
}

#[test]
fn params_map_keeps_false_return_documents() {
    assert_eq!(
        mapped(r#"{"return_documents":false}"#),
        params(r#"{"parameters":{"return_options":{"inputs":false}}}"#)
    );
}

#[test]
fn params_map_ignores_non_bool_return_documents() {
    assert_eq!(mapped(r#"{"return_documents":"yes"}"#), Params::new());
}

#[test]
fn params_map_keeps_document_objects_as_given() {
    assert_eq!(
        mapped(r#"{"documents":[{"text":"x"},{"a":1}]}"#),
        params(r#"{"inputs":[{"text":"x"},{"a":1}]}"#)
    );
}

#[rstest]
#[case::text(r#"{"documents":"abc"}"#)]
#[case::number(r#"{"documents":5}"#)]
#[case::object(r#"{"documents":{"text":"a"}}"#)]
#[case::document_number(r#"{"documents":[1]}"#)]
#[case::document_null(r#"{"documents":[null]}"#)]
#[case::document_list(r#"{"documents":[["a"]]}"#)]
fn params_map_rejects_wrong_document_shapes(#[case] value: &str) {
    let error = map(&mut params(value)).unwrap_err();
    assert!(matches!(error, WatsonxRerankError::InvalidDocuments));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::version_used(r#"{"api_version":"v"}"#, "https://host/ml/v1/text/rerank?version=v")]
#[case::empty_version(
    r#"{"api_version":""}"#,
    "https://host/ml/v1/text/rerank?version=2024-03-13"
)]
#[case::no_version("{}", "https://host/ml/v1/text/rerank?version=2024-03-13")]
#[case::null_version(
    r#"{"api_version":null}"#,
    "https://host/ml/v1/text/rerank?version=2024-03-13"
)]
fn url_reads_version_without_popping(#[case] wire: &str, #[case] expected: &str) {
    let auth = auth(WatsonxEnvironment::default());
    let options = params(wire);
    let before = options.clone();
    let url = config(&auth)
        .get_complete_url(Some("https://host/"), Some(&options))
        .unwrap();
    assert_eq!(url, expected);
    assert_eq!(options, before);
}

#[test]
fn url_falls_back_to_environment_base() {
    let auth = auth(environment(&[("WATSONX_API_BASE", "https://env-host")]));
    assert_eq!(
        config(&auth).get_complete_url(None, None).unwrap(),
        "https://env-host/ml/v1/text/rerank?version=2024-03-13"
    );
}

#[test]
fn url_without_base_is_401() {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth).get_complete_url(None, None).unwrap_err();
    assert_eq!(error.status(), Some(401));
}

#[test]
fn url_with_non_text_version_names_the_field() {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth)
        .get_complete_url(Some("https://host"), Some(&params(r#"{"api_version":5}"#)))
        .unwrap_err();
    assert!(matches!(
        error,
        WatsonxRerankError::NotText {
            field: "api_version"
        }
    ));
}

#[tokio::test]
async fn auth_returns_defaults_with_authorization() {
    let auth = auth(WatsonxEnvironment::default());
    let mut options = params(r#"{"token":"t","project_id":"p"}"#);
    let resolved = config(&auth)
        .validate_environment(
            headers(&[("Authorization", "Bearer given"), ("X-Extra", "1")]),
            None,
            &mut options,
        )
        .await
        .unwrap();
    let expected = headers(&[
        ("Accept", "application/json"),
        ("Authorization", "Bearer given"),
        ("Content-Type", "application/json"),
        ("X-Extra", "1"),
    ]);
    assert_eq!(resolved, expected);
    assert_eq!(options, params(r#"{"token":"t","project_id":"p"}"#));
}

#[tokio::test]
async fn auth_uses_token_and_pops_both_auth_params() {
    let auth = auth(WatsonxEnvironment::default());
    let mut options = params(r#"{"token":"t","zen_api_key":"z","project_id":"p"}"#);
    let resolved = config(&auth)
        .validate_environment(Headers::new(), None, &mut options)
        .await
        .unwrap();
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer t")
    );
    assert_eq!(options, params(r#"{"project_id":"p"}"#));
}

#[tokio::test]
async fn auth_uses_environment_token() {
    let auth = auth(environment(&[("WATSONX_TOKEN", "envtoken")]));
    let mut options = params(r#"{"zen_api_key":"z"}"#);
    let resolved = config(&auth)
        .validate_environment(Headers::new(), None, &mut options)
        .await
        .unwrap();
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer envtoken")
    );
    assert_eq!(options, Params::new());
}

// an empty env token or zen key counts as unset, so the IAM fallback runs and fails without a key
#[tokio::test]
async fn auth_empty_environment_values_fall_through() {
    let auth = auth(environment(&[
        ("WATSONX_TOKEN", ""),
        ("WATSONX_ZENAPIKEY", ""),
    ]));
    let mut options = Params::new();
    let error = config(&auth)
        .validate_environment(Headers::new(), None, &mut options)
        .await
        .unwrap_err();
    assert!(matches!(error, WatsonxRerankError::Watsonx(_)));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::option(r#"{"zen_api_key":"z"}"#, None, "ZenApiKey z")]
#[case::option_after_null_token(r#"{"token":null,"zen_api_key":"z"}"#, None, "ZenApiKey z")]
#[case::empty_token_falls_through(r#"{"token":""}"#, Some(("WATSONX_ZENAPIKEY", "envzen")), "ZenApiKey envzen")]
#[case::environment( "{}", Some(("WATSONX_ZENAPIKEY", "envzen")), "ZenApiKey envzen")]
#[tokio::test]
async fn auth_uses_zen_api_key(
    #[case] wire: &str,
    #[case] variable: Option<(&str, &str)>,
    #[case] expected: &str,
) {
    let values: Vec<(&str, &str)> = variable.into_iter().collect();
    let auth = auth(environment(&values));
    let mut options = params(wire);
    let resolved = config(&auth)
        .validate_environment(Headers::new(), None, &mut options)
        .await
        .unwrap();
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some(expected)
    );
    assert_eq!(options, Params::new());
}

#[tokio::test]
async fn auth_without_any_key_is_missing_iam_key() {
    let auth = auth(WatsonxEnvironment::default());
    let mut options = Params::new();
    let error = config(&auth)
        .validate_environment(Headers::new(), None, &mut options)
        .await
        .unwrap_err();
    assert!(matches!(error, WatsonxRerankError::Watsonx(_)));
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn auth_rejects_non_text_token() {
    let auth = auth(WatsonxEnvironment::default());
    let mut options = params(r#"{"token":5}"#);
    let error = config(&auth)
        .validate_environment(Headers::new(), None, &mut options)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        WatsonxRerankError::NotText { field: "token" }
    ));
}

#[tokio::test]
async fn auth_rejects_non_text_zen_api_key() {
    let auth = auth(WatsonxEnvironment::default());
    let mut options = params(r#"{"zen_api_key":[]}"#);
    let error = config(&auth)
        .validate_environment(Headers::new(), None, &mut options)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        WatsonxRerankError::NotText {
            field: "zen_api_key"
        }
    ));
}

#[test]
fn request_pops_ids_and_adds_payload() {
    assert_eq!(
        transform_rerank_request(
            "granite",
            &params(
                r#"{"query":"q","inputs":["a"],"project_id":"p","space_id":"s","api_version":"v"}"#
            ),
            &WatsonxEnvironment::default(),
        )
        .unwrap(),
        params(
            r#"{"query":"q","inputs":["a"],"api_version":"v","model_id":"granite","project_id":"p"}"#
        )
    );
}

#[test]
fn request_for_deployment_model_keeps_params() {
    assert_eq!(
        transform_rerank_request(
            "deployment/d",
            &params(r#"{"query":"q","space_id":"s"}"#),
            &WatsonxEnvironment::default(),
        )
        .unwrap(),
        params(r#"{"query":"q"}"#)
    );
}

#[test]
fn request_without_ids_is_401() {
    let error = transform_rerank_request(
        "granite",
        &params(r#"{"query":"q"}"#),
        &WatsonxEnvironment::default(),
    )
    .unwrap_err();
    assert_eq!(error.status(), Some(401));
}

#[test]
fn response_maps_results_and_meta() {
    let auth = auth(WatsonxEnvironment::default());
    let body = br#"{"results":[{"index":1,"score":0.25,"input":"hello"},{"index":0,"score":0.5,"input":{"text":"t","x":1}}],"id":"rid","input_token_count":4}"#;
    assert_eq!(
        config(&auth)
            .transform_rerank_response(&response(200, body))
            .unwrap(),
        RerankResponse {
            id: Some(String::from("rid")),
            results: Some(vec![
                RerankResponseResult {
                    index: 1,
                    relevance_score: score(0.25),
                    document: Some(RerankResponseDocument {
                        text: String::from("hello"),
                    }),
                },
                RerankResponseResult {
                    index: 0,
                    relevance_score: score(0.5),
                    document: Some(RerankResponseDocument {
                        text: String::from("t"),
                    }),
                },
            ]),
            meta: Some(meta(RerankTokens {
                input_tokens: Some(Some(4)),
                output_tokens: None,
            })),
            hidden_params: BTreeMap::new(),
        }
    );
}

#[test]
fn response_without_id_generates_one() {
    let auth = auth(WatsonxEnvironment::default());
    let response = config(&auth)
        .transform_rerank_response(&response(200, br#"{"results":[]}"#))
        .unwrap();
    assert_eq!(response.id, Some(Uuid::nil().to_string()));
    assert_eq!(response.results, Some(Vec::new()));
    assert_eq!(
        response.meta,
        Some(meta(RerankTokens {
            input_tokens: Some(Some(0)),
            output_tokens: None,
        }))
    );
}

#[rstest]
#[case::empty_id(br#"{"results":[],"id":""}"#)]
#[case::null_id(br#"{"results":[],"id":null}"#)]
fn response_blank_id_generates_one(#[case] body: &[u8]) {
    let auth = auth(WatsonxEnvironment::default());
    let response = config(&auth)
        .transform_rerank_response(&response(200, body))
        .unwrap();
    assert_eq!(response.id, Some(Uuid::nil().to_string()));
}

#[test]
fn response_null_token_count_stays_null() {
    let auth = auth(WatsonxEnvironment::default());
    let response = config(&auth)
        .transform_rerank_response(&response(
            200,
            br#"{"results":[],"input_token_count":null}"#,
        ))
        .unwrap();
    assert_eq!(
        response.meta,
        Some(meta(RerankTokens {
            input_tokens: Some(None),
            output_tokens: None,
        }))
    );
}

#[test]
fn response_missing_input_has_no_document() {
    let auth = auth(WatsonxEnvironment::default());
    let response = config(&auth)
        .transform_rerank_response(&response(200, br#"{"results":[{"index":0,"score":0.25}]}"#))
        .unwrap();
    let results = response.results.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results.first().unwrap().document, None);
}

fn failure(body: &[u8]) -> WatsonxRerankError {
    let auth = auth(WatsonxEnvironment::default());
    config(&auth)
        .transform_rerank_response(&response(200, body))
        .unwrap_err()
}

#[rstest]
#[case::missing_index(
    br#"{"results":[{"score":0.25}]}"#,
    "the rerank reply does not match the record shape: "
)]
#[case::fractional_index(
    br#"{"results":[{"index":1.5,"score":0.25}]}"#,
    "the rerank reply does not match the record shape: "
)]
#[case::missing_score(
    br#"{"results":[{"index":0}]}"#,
    "the rerank reply does not match the record shape: "
)]
#[case::text_score(
    br#"{"results":[{"index":0,"score":"high"}]}"#,
    "the rerank reply does not match the record shape: "
)]
#[case::bad_id(
    br#"{"results":[],"id":7}"#,
    "the rerank reply does not match the record shape: "
)]
#[case::fractional_tokens(
    br#"{"results":[],"input_token_count":7.5}"#,
    "the rerank reply does not match the record shape: "
)]
#[case::results_object(
    br#"{"results":{"index":0}}"#,
    "the rerank reply does not match the record shape: "
)]
#[case::not_an_object(b"[]", "the rerank reply does not match the record shape: ")]
#[case::null_input(
    br#"{"results":[{"index":0,"score":0.25,"input":null}]}"#,
    "the reply field results[].input has the wrong shape"
)]
#[case::number_input(
    br#"{"results":[{"index":0,"score":0.25,"input":5}]}"#,
    "the reply field results[].input has the wrong shape"
)]
#[case::list_input(
    br#"{"results":[{"index":0,"score":0.25,"input":["a"]}]}"#,
    "the reply field results[].input has the wrong shape"
)]
#[case::object_without_text(
    br#"{"results":[{"index":0,"score":0.25,"input":{"a":1}}]}"#,
    "the reply field results[].input.text has the wrong shape"
)]
#[case::object_with_non_text(
    br#"{"results":[{"index":0,"score":0.25,"input":{"text":5}}]}"#,
    "the reply field results[].input.text has the wrong shape"
)]
fn response_rejects_wrong_shapes(#[case] body: &[u8], #[case] message: &str) {
    let error = failure(body);
    assert!(error.to_string().starts_with(message));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::missing_results(b"{}")]
#[case::null_results(br#"{"results":null}"#)]
fn response_without_results_fails(#[case] body: &[u8]) {
    let error = failure(body);
    assert!(matches!(error, WatsonxRerankError::NoResults { .. }));
    assert_eq!(
        error.to_string(),
        format!(
            "No results found in the response={}",
            str::from_utf8(body).unwrap()
        )
    );
}

#[test]
fn response_non_json_uses_the_response_status() {
    let auth = auth(WatsonxEnvironment::default());
    let error = config(&auth)
        .transform_rerank_response(&response(502, b"not json"))
        .unwrap_err();
    assert_eq!(error.status(), Some(502));
    assert!(matches!(error, WatsonxRerankError::Watsonx(_)));
    assert!(error.to_string().contains("Failed to parse response"));
}

#[test]
fn parse_error_is_available_from_the_decoder() {
    let error = super::reply::decode_watsonx_rerank_reply(b"not json").unwrap_err();
    assert!(matches!(error, WatsonxRerankError::Parse(_)));
    assert_eq!(error.status(), None);
}
