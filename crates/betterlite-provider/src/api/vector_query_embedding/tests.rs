#![cfg(test)]

use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreSearchResponse,
};
use betterlite_openai::embedding::EmbeddingResponse;
use rstest::rstest;
use serde_json::Number;
use thiserror::Error;

use crate::api::vector_store_contracts::config::{
    AuthCredentials, EmbeddingEntry, EndpointsByType, Params, PreparedRequest, SearchQuery,
    SearchRequest, VectorStoreConfig, VectorStoreEmbeddingExecutor, VectorStoreLogging,
    VectorStoreResponse,
};
use crate::api::vector_store_contracts::error::{
    VectorStoreContractError, VectorStoreEmbeddingError,
};

use super::direct::{
    DirectSearchRequest, DirectVectorStoreConfig, NO_REQUEST_SHAPE, NO_RESPONSE_SHAPE,
};
use super::error::{DirectStoreError, QueryEmbeddingError};
use super::query::{
    QueryEmbeddingConfig, QuerySearchRequest, embedding_configuration, first_embedding_vector,
    query_embedding_configuration, query_embedding_model, query_text,
};

#[derive(Debug, Error)]
enum FixtureError {
    #[error(transparent)]
    Contract(#[from] VectorStoreContractError),
}

fn contract_error() -> FixtureError {
    FixtureError::Contract(VectorStoreContractError::ApiBaseRequired)
}

fn params(text: &str) -> Params {
    serde_json::from_str(text).expect("params")
}

fn records(text: &str) -> Vec<Json> {
    serde_json::from_str(text).expect("records")
}

fn response() -> VectorStoreResponse {
    VectorStoreResponse {
        status_code: 200,
        headers: BTreeMap::new(),
        body: Vec::new(),
    }
}

#[derive(Debug, Default)]
struct DirectFixture;

impl DirectVectorStoreConfig for DirectFixture {
    fn execute_search_request(
        &self,
        _request: DirectSearchRequest<'_>,
    ) -> Result<VectorStoreSearchResponse, DirectStoreError> {
        Ok(VectorStoreSearchResponse::default())
    }
}

#[rstest]
#[case::text(" a ")]
#[case::padded("a b")]
fn query_text_keeps_a_text_query(#[case] query: &str) {
    assert_eq!(query_text(&SearchQuery::Text(query.to_owned())), query);
}

#[rstest]
#[case::two(vec!["a".to_owned(), "b".to_owned()], "a b")]
#[case::empty(vec![], "")]
#[case::holes(
    vec!["a".to_owned(), String::new(), "b".to_owned()],
    "a  b"
)]
fn query_text_joins_a_sequence_with_one_space(#[case] query: Vec<String>, #[case] expected: &str) {
    assert_eq!(query_text(&SearchQuery::Sequence(query)), expected);
}

#[rstest]
#[case::model(
    r#"{"litellm_embedding_model": "openai/text-embedding-3-small"}"#,
    "openai/text-embedding-3-small"
)]
#[case::space(r#"{"litellm_embedding_model": " "}"#, " ")]
fn query_embedding_model_accepts_a_nonempty_text(
    #[case] configuration: &str,
    #[case] expected: &str,
) {
    assert!(matches!(
        query_embedding_model(&params(configuration)),
        Ok(model) if model == expected
    ));
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"litellm_embedding_model": null}"#)]
#[case::empty(r#"{"litellm_embedding_model": ""}"#)]
#[case::number(r#"{"litellm_embedding_model": 1}"#)]
fn query_embedding_model_refuses_anything_else(#[case] configuration: &str) {
    assert!(matches!(
        query_embedding_model(&params(configuration)),
        Err(QueryEmbeddingError::MissingModel)
    ));
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"litellm_embedding_config": null}"#)]
#[case::list(r#"{"litellm_embedding_config": [1, 2]}"#)]
fn query_embedding_configuration_takes_only_mappings(#[case] litellm_params: &str) {
    assert_eq!(
        query_embedding_configuration(&params(litellm_params)).expect("configuration"),
        Params::new()
    );
}

#[test]
fn query_embedding_configuration_keeps_a_mapping() {
    let configured = query_embedding_configuration(&params(
        r#"{"litellm_embedding_config": {"dimensions": 2}}"#,
    ))
    .expect("configuration");
    assert_eq!(configured, params(r#"{"dimensions": 2}"#));
}

#[test]
fn embedding_configuration_writes_an_integer_key_as_decimal() {
    assert_eq!(
        embedding_configuration(vec![
            (
                Json::Number(Number::from(1_u8)),
                Json::Text("one".to_owned())
            ),
            (
                Json::Text("dimensions".to_owned()),
                Json::Number(Number::from(2_u8))
            ),
        ])
        .expect("configuration"),
        params(r#"{"1": "one", "dimensions": 2}"#)
    );
    assert!(matches!(
        embedding_configuration(vec![(Json::Bool(true), Json::Null)]),
        Err(QueryEmbeddingError::ConfigKey)
    ));
}

#[rstest]
#[case::first_of_two(
    r#"[{"embedding": [0.25, 0.5]}, {"embedding": [9, 9]}]"#,
    vec![0.25_f64, 0.5_f64]
)]
#[case::empty_vector(r#"[{"embedding": []}]"#, vec![])]
#[case::numeric_text(r#"[{"embedding": ["0.5", "1"]}]"#, vec![0.5_f64, 1.0_f64])]
fn first_embedding_vector_reads_only_the_first_record(
    #[case] data: &str,
    #[case] expected: Vec<f64>,
) {
    assert_eq!(first_embedding_vector(&records(data)).unwrap(), expected);
}

#[rstest]
#[case::no_data("[]", 0_u8)]
#[case::no_embedding("[{}]", 1_u8)]
#[case::not_a_list(r#"[{"embedding": "abc"}]"#, 2_u8)]
#[case::not_a_number(r#"[{"embedding": ["bad"]}]"#, 2_u8)]
fn first_embedding_vector_refuses_a_bad_record(#[case] data: &str, #[case] expected: u8) {
    let error = first_embedding_vector(&records(data)).unwrap_err();
    let kind = match error {
        QueryEmbeddingError::NoFirstRecord => 0,
        QueryEmbeddingError::NoEmbedding => 1,
        QueryEmbeddingError::NotAVector => 2,
        QueryEmbeddingError::MissingModel
        | QueryEmbeddingError::ConfigKey
        | QueryEmbeddingError::Execution { .. } => 3,
    };
    assert_eq!(kind, expected);
}

#[test]
fn direct_store_defaults_match_litellm() {
    let fixture = DirectFixture;
    let empty = Params::new();
    assert_eq!(fixture.complete_url(None, &empty).unwrap(), String::new());
    assert_eq!(
        fixture.complete_url(Some(""), &empty).unwrap(),
        String::new()
    );
    assert_eq!(
        fixture
            .complete_url(Some("https://example.test/path"), &empty)
            .unwrap(),
        "https://example.test/path"
    );
    assert_eq!(
        serde_json::to_string(&fixture.auth_credentials(&empty)).unwrap(),
        "{}"
    );
    assert_eq!(
        serde_json::to_string(&fixture.endpoints_by_type()).unwrap(),
        r#"{"read":[],"write":[]}"#
    );
    let mut logging = VectorStoreLogging::default();
    let executed = fixture
        .execute_search_request(DirectSearchRequest {
            vector_store_id: "vs_a",
            query: &SearchQuery::Text("hello".to_owned()),
            optional_params: &empty,
            logging: &mut logging,
            litellm_params: &empty,
            embedding_executor: None,
            timeout: None,
        })
        .expect("the store executes its own search");
    assert_eq!(executed, VectorStoreSearchResponse::default());
}

fn not_implemented_message<T>(outcome: &Result<T, DirectStoreError>) -> &'static str {
    match outcome {
        Err(DirectStoreError::NotImplemented { message }) => message,
        Err(DirectStoreError::Contract(error)) => panic!("unexpected contract error {error}"),
        Ok(_) => panic!("the direct store must not transform an HTTP request"),
    }
}

#[test]
fn direct_store_http_transforms_are_not_implemented() {
    let fixture = DirectFixture;
    let mut request_logging = VectorStoreLogging::default();
    let request = SearchRequest {
        vector_store_id: "vs_a",
        query: &SearchQuery::Text("hello".to_owned()),
        optional_params: &Params::new(),
        api_base: "https://example.test",
        logging: &mut request_logging,
        litellm_params: &Params::new(),
        extra_body: None,
    };
    assert_eq!(
        not_implemented_message(&fixture.transform_search_request(request)),
        NO_REQUEST_SHAPE
    );
    let mut response_logging = VectorStoreLogging::default();
    assert_eq!(
        not_implemented_message(
            &fixture.transform_search_response(&response(), &mut response_logging)
        ),
        NO_RESPONSE_SHAPE
    );
    assert_eq!(
        not_implemented_message(
            &fixture.transform_create_request(&Params::new(), "https://a.test")
        ),
        ""
    );
    assert_eq!(
        not_implemented_message(&fixture.transform_create_response(&response())),
        ""
    );
}

#[derive(Debug, Default)]
struct QueryFixture;

impl VectorStoreConfig for QueryFixture {
    type Error = FixtureError;

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        AuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::default()
    }

    fn transform_search_request(
        &self,
        _request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(contract_error())
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, FixtureError> {
        Err(contract_error())
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(contract_error())
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, FixtureError> {
        Err(contract_error())
    }
}

struct RecordedExecutor;

#[async_trait]
impl VectorStoreEmbeddingExecutor for RecordedExecutor {
    async fn embed(
        &self,
        _entry: EmbeddingEntry,
        _model: &str,
        _query: &str,
        _configuration: &Params,
    ) -> Result<EmbeddingResponse, VectorStoreEmbeddingError> {
        Err(VectorStoreEmbeddingError::DuplicateArgument {
            name: "the forwarding fixture never runs".to_owned(),
        })
    }
}

fn entries(parameters: &Params) -> Json {
    Json::Object(
        parameters
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

fn forwarded_body(search: &SearchRequest<'_>, has_executor: bool) -> Params {
    let query = match search.query {
        SearchQuery::Text(text) => Json::Text(text.clone()),
        SearchQuery::Sequence(parts) => {
            Json::List(parts.iter().map(|part| Json::Text(part.clone())).collect())
        }
    };
    let executor = if has_executor {
        Json::Text("explicit".to_owned())
    } else {
        Json::Null
    };
    Params::from([
        (
            "vector_store_id".to_owned(),
            Json::Text(search.vector_store_id.to_owned()),
        ),
        ("query".to_owned(), query),
        (
            "optional_params".to_owned(),
            entries(search.optional_params),
        ),
        (
            "api_base".to_owned(),
            Json::Text(search.api_base.to_owned()),
        ),
        ("litellm_params".to_owned(), entries(search.litellm_params)),
        (
            "extra_body".to_owned(),
            search.extra_body.map_or(Json::Null, entries),
        ),
        ("embedding_executor".to_owned(), executor),
    ])
}

// the fixture records one call in the logging object LiteLLM uses for it
fn record_call(logging: &mut VectorStoreLogging) {
    let calls = match logging.model_call_details.get("calls") {
        Some(Json::Number(number)) => number.as_u64().unwrap_or(0),
        _ => 0,
    }
    .saturating_add(1);
    logging
        .model_call_details
        .insert("calls".to_owned(), Json::Number(Number::from(calls)));
}

impl QueryFixture {
    fn record(request: QuerySearchRequest<'_>) -> PreparedRequest {
        let QuerySearchRequest {
            search,
            embedding_executor,
        } = request;
        let body = forwarded_body(&search, embedding_executor.is_some());
        record_call(search.logging);
        (format!("{}/vector_stores/search", search.api_base), body)
    }
}

impl QueryEmbeddingConfig for QueryFixture {
    fn transform_query_search_request(
        &self,
        request: QuerySearchRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(Self::record(request))
    }
}

#[derive(Debug)]
struct Fixtures {
    optional: Params,
    litellm: Params,
    extra: Params,
    query: SearchQuery,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            optional: params(r#"{"max_num_results": 3}"#),
            litellm: params(r#"{"litellm_embedding_model": "openai/text-embedding-3-small"}"#),
            extra: params(r#"{"e": true}"#),
            query: SearchQuery::Sequence(vec!["a".to_owned(), String::new()]),
        }
    }

    fn request<'request>(
        &'request self,
        logging: &'request mut VectorStoreLogging,
        executor: &'request RecordedExecutor,
    ) -> QuerySearchRequest<'request> {
        QuerySearchRequest {
            search: SearchRequest {
                vector_store_id: "vs_a",
                query: &self.query,
                optional_params: &self.optional,
                api_base: "https://example.test",
                logging,
                litellm_params: &self.litellm,
                extra_body: Some(&self.extra),
            },
            embedding_executor: Some(executor),
        }
    }
}

fn calls(logging: &VectorStoreLogging) -> Option<&Json> {
    logging.model_call_details.get("calls")
}

#[tokio::test]
async fn async_query_transform_forwards_every_argument_once() {
    let fixture = QueryFixture;
    let fixtures = Fixtures::new();
    let executor = RecordedExecutor;
    let mut sync_logging = VectorStoreLogging::default();
    let sync = fixture
        .transform_query_search_request(fixtures.request(&mut sync_logging, &executor))
        .expect("sync");
    let mut async_logging = VectorStoreLogging::default();
    let asynchronous = fixture
        .transform_query_search_async(fixtures.request(&mut async_logging, &executor))
        .await
        .expect("async");
    assert_eq!(sync, asynchronous);
    assert_eq!(sync.0, "https://example.test/vector_stores/search");
    assert_eq!(
        sync.1.get("query"),
        Some(&Json::List(vec![
            Json::Text("a".to_owned()),
            Json::Text(String::new()),
        ]))
    );
    assert_eq!(
        sync.1.get("embedding_executor"),
        Some(&Json::Text("explicit".to_owned()))
    );
    assert_eq!(sync.1.get("extra_body"), Some(&entries(&fixtures.extra)));
    let one = Json::Number(Number::from(1_u8));
    assert_eq!(calls(&sync_logging), Some(&one));
    assert_eq!(calls(&async_logging), Some(&one));
}
