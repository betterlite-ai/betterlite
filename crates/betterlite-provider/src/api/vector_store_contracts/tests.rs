#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreSearchResponse,
};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use thiserror::Error;

use super::config::{
    AuthCredentials, EndpointsByType, Headers, Params, PreparedRequest, SearchQuery, SearchRequest,
    SignRequest, VectorStoreConfig, VectorStoreLogging, VectorStoreResponse,
};
use super::error::{BaseLlmException, VectorStoreContractError};

#[derive(Debug, Default)]
struct Fixture;

#[derive(Debug, Error)]
enum FixtureError {
    #[error(transparent)]
    Contract(#[from] VectorStoreContractError),
}

fn prepared_error() -> FixtureError {
    FixtureError::Contract(VectorStoreContractError::ApiBaseRequired)
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn params(text_body: &str) -> Params {
    serde_json::from_str(text_body).expect("params")
}

fn object(params: &Params) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

impl VectorStoreConfig for Fixture {
    type Error = FixtureError;

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        AuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::default()
    }

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        let calls = match request.logging.model_call_details.get("calls") {
            Some(Json::Number(number)) => number.as_u64().unwrap_or_default(),
            _ => 0,
        };
        request.logging.model_call_details.insert(
            "calls".to_owned(),
            Json::Number(Number::from(calls.saturating_add(1))),
        );
        let query = match request.query {
            SearchQuery::Text(value) => text(value),
            SearchQuery::Sequence(values) => {
                Json::List(values.iter().map(|value| text(value)).collect())
            }
        };
        let body = Params::from([
            ("vector_store_id".to_owned(), text(request.vector_store_id)),
            ("query".to_owned(), query),
            (
                "optional_params".to_owned(),
                object(request.optional_params),
            ),
            ("api_base".to_owned(), text(request.api_base)),
            ("litellm_params".to_owned(), object(request.litellm_params)),
            (
                "extra_body".to_owned(),
                request.extra_body.map_or(Json::Null, object),
            ),
        ]);
        Ok((format!("{}/vector_stores/search", request.api_base), body))
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, Self::Error> {
        Err(prepared_error())
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(prepared_error())
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, Self::Error> {
        Err(prepared_error())
    }
}

#[test]
fn validate_create_vector_store_succeeds() {
    Fixture.validate_create_vector_store();
}

#[test]
fn supported_params_are_empty() {
    assert!(
        Fixture
            .supported_params("openai/text-embedding-3-small")
            .is_empty()
    );
}

#[rstest]
#[case::drop_params(true)]
#[case::keep_params(false)]
fn map_params_returns_optional_params_unchanged(#[case] drop_params: bool) {
    let optional = params(r#"{"filters": {}, "max_num_results": 0}"#);
    let non_default = params(r#"{"filters": {}, "ranking_options": null, "unknown": 1}"#);
    let mapped = Fixture.map_params(&non_default, optional.clone(), drop_params);
    assert_eq!(mapped, optional);
}

#[test]
fn validate_environment_returns_no_headers() {
    let incoming = Headers::from([("Authorization".to_owned(), "Bearer x".to_owned())]);
    assert!(Fixture.validate_environment(&incoming, None).is_empty());
}

#[rstest]
#[case::explicit_empty(Some(""), "")]
#[case::supplied(Some("https://example.test/path"), "https://example.test/path")]
fn complete_url_returns_the_supplied_base(#[case] api_base: Option<&str>, #[case] expected: &str) {
    let url = Fixture.complete_url(api_base, &Params::new()).expect("url");
    assert_eq!(url, expected);
}

#[test]
fn complete_url_requires_an_api_base() {
    let error = Fixture
        .complete_url(None, &Params::new())
        .expect_err("missing base");
    assert_eq!(error.to_string(), "api_base is required");
}

#[test]
fn sign_request_returns_the_same_headers_and_no_body() {
    let headers = Headers::from([("X-Key".to_owned(), "v".to_owned())]);
    let api_key = SecretString::from("key");
    let (signed, body) = Fixture.sign_request(SignRequest {
        headers: &headers,
        optional_params: &params(r#"{"a": 1}"#),
        request_data: &params(r#"{"b": 2}"#),
        api_base: "https://example.test",
        api_key: Some(&api_key),
    });
    assert_eq!(signed, headers);
    assert_eq!(body, None);
}

#[test]
fn error_class_retains_status_message_and_headers() {
    let headers = params(r#"{"X-Test": "x"}"#);
    let raised: BaseLlmException = Fixture.error_class("denied", 403, headers.clone());
    assert_eq!(raised.status_code, 403);
    assert_eq!(raised.message, "denied");
    assert_eq!(raised.headers, headers);
    assert_eq!(raised.to_string(), "denied");
}

#[test]
fn calculate_vector_store_cost_is_zero() {
    let response = VectorStoreSearchResponse::default();
    let (prompt, completion) = Fixture.calculate_vector_store_cost(&response);
    assert_eq!((prompt.is_zero(), completion.is_zero()), (true, true));
}

fn search_inputs() -> (Params, Params, Params) {
    (
        params(r#"{"filters": {"x": 1}}"#),
        params(r#"{"api_key": "k"}"#),
        params(r#"{"e": true}"#),
    )
}

fn search_request<'request>(
    query: &'request SearchQuery,
    optional: &'request Params,
    litellm_params: &'request Params,
    extra_body: &'request Params,
    logging: &'request mut VectorStoreLogging,
) -> SearchRequest<'request> {
    SearchRequest {
        vector_store_id: "vs_a",
        query,
        optional_params: optional,
        api_base: "https://example.test",
        logging,
        litellm_params,
        extra_body: Some(extra_body),
    }
}

fn assert_forwarded_pair(pair: &PreparedRequest, optional: &Params, extra_body: &Params) {
    let (url, body) = pair;
    assert_eq!(url, "https://example.test/vector_stores/search");
    assert_eq!(body.get("vector_store_id"), Some(&text("vs_a")));
    assert_eq!(body.get("query"), Some(&text("hello")));
    assert_eq!(body.get("api_base"), Some(&text("https://example.test")));
    assert_eq!(body.get("optional_params"), Some(&object(optional)));
    assert_eq!(body.get("extra_body"), Some(&object(extra_body)));
}

#[tokio::test]
async fn async_search_transformation_forwards_once_and_returns_the_sync_pair() {
    let (optional, litellm_params, extra_body) = search_inputs();
    let query = SearchQuery::Text(String::from("hello"));

    let mut sync_logging = VectorStoreLogging::default();
    let sync = Fixture
        .transform_search_request(search_request(
            &query,
            &optional,
            &litellm_params,
            &extra_body,
            &mut sync_logging,
        ))
        .expect("sync transform");
    let mut async_logging = VectorStoreLogging::default();
    let asynchronous = Fixture
        .transform_search_request_async(search_request(
            &query,
            &optional,
            &litellm_params,
            &extra_body,
            &mut async_logging,
        ))
        .await
        .expect("async transform");

    assert_eq!(asynchronous, sync);
    let one = Some(Json::Number(Number::from(1_u8)));
    assert_eq!(sync_logging.model_call_details.get("calls"), one.as_ref());
    assert_eq!(async_logging.model_call_details.get("calls"), one.as_ref());
    assert_forwarded_pair(&asynchronous, &optional, &extra_body);
}

#[test]
fn search_query_keeps_text_and_sequence_apart() {
    let cases: Vec<SearchQuery> = vec![
        SearchQuery::Text(String::from("q")),
        SearchQuery::Sequence(vec![String::from("q"), String::from("r")]),
    ];
    let json: Vec<Json> = cases
        .iter()
        .map(|query| {
            serde_json::from_value(serde_json::to_value(query).expect("query")).expect("json")
        })
        .collect();
    assert_eq!(
        json,
        vec![text("q"), Json::List(vec![text("q"), text("r")])]
    );
}
