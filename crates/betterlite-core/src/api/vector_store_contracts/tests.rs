#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{ModelDeployment, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreSearchResponse,
};
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_provider::api::vector_store_contracts::config::{
    AuthCredentials, EndpointsByType, Params, PreparedRequest, SearchRequest, VectorStoreConfig,
    VectorStoreEmbeddingExecutor as _, VectorStoreLogging, VectorStoreResponse,
};
use betterlite_provider::api::vector_store_contracts::error::{
    BaseLlmException, VectorStoreContractError, VectorStoreEmbeddingError,
};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde_json::Number;
use thiserror::Error;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::executor::DirectEmbeddingExecutor;
use super::filters::{requested_create_params, requested_search_params};
use super::metadata::{RequestMetadataError, metadata_map, vector_store_request_metadata};
use super::router::{RouterEmbeddingExecutor, VectorStoreRouter};

const EMBEDDING_ANSWER: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,0.5]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":1,"total_tokens":1}}"#;

fn params(text_body: &str) -> Params {
    serde_json::from_str(text_body).expect("params")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[derive(Debug, Default)]
struct DefaultConfig;

#[derive(Debug, Error)]
enum FixtureError {
    #[error(transparent)]
    Contract(#[from] VectorStoreContractError),
}

fn unsupported() -> FixtureError {
    FixtureError::Contract(VectorStoreContractError::ApiBaseRequired)
}

impl VectorStoreConfig for DefaultConfig {
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
    ) -> Result<PreparedRequest, Self::Error> {
        Err(unsupported())
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, Self::Error> {
        Err(unsupported())
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(unsupported())
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, Self::Error> {
        Err(unsupported())
    }
}

// stands in for a provider mapper: it reports what it was given and adds a field of its own
struct RecordingConfig;

impl VectorStoreConfig for RecordingConfig {
    type Error = FixtureError;

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        AuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::default()
    }

    fn map_params(
        &self,
        non_default_params: &Params,
        mut optional_params: Params,
        drop_params: bool,
    ) -> Params {
        let observed = Params::from([
            (
                "non_default_params".to_owned(),
                Json::Object(
                    non_default_params
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect(),
                ),
            ),
            ("drop_params".to_owned(), Json::Bool(drop_params)),
        ]);
        optional_params.insert(
            "observed".to_owned(),
            Json::Object(observed.into_iter().collect()),
        );
        optional_params
    }

    fn transform_search_request(
        &self,
        _request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(unsupported())
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, Self::Error> {
        Err(unsupported())
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(unsupported())
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, Self::Error> {
        Err(unsupported())
    }
}

#[test]
fn optional_parameter_filters_keep_declared_values_and_drop_nulls_and_unknowns() {
    let create = params(
        r#"{"name": "", "file_ids": [], "expires_after": null, "chunking_strategy": {"type": "auto"}, "metadata": {}, "unknown": 1}"#,
    );
    assert_eq!(
        requested_create_params(&create),
        params(
            r#"{"name": "", "file_ids": [], "chunking_strategy": {"type": "auto"}, "metadata": {}}"#
        )
    );
    let search = params(
        r#"{"filters": {}, "max_num_results": 0, "ranking_options": null, "rewrite_query": false, "unknown": 1}"#,
    );
    assert_eq!(
        requested_search_params(&search, &DefaultConfig),
        params(r#"{"filters": {}, "max_num_results": 0, "rewrite_query": false}"#)
    );
}

#[test]
fn search_mapper_sees_the_whole_request_and_keeps_its_own_keys() {
    let search = params(
        r#"{"filters": {}, "max_num_results": 0, "ranking_options": null, "rewrite_query": false, "unknown": 1}"#,
    );
    let mapped = requested_search_params(&search, &RecordingConfig);
    assert_eq!(
        mapped,
        params(
            r#"{"filters": {}, "max_num_results": 0, "rewrite_query": false, "observed": {"non_default_params": {"filters": {}, "max_num_results": 0, "ranking_options": null, "rewrite_query": false, "unknown": 1}, "drop_params": false}}"#
        )
    );
}

#[rstest]
#[case::missing_fields("{}", "{}")]
#[case::zero_is_kept(r#"{"max_num_results": 0}"#, r#"{"max_num_results": 0}"#)]
#[case::negative_is_kept(r#"{"max_num_results": -1}"#, r#"{"max_num_results": -1}"#)]
#[case::null_is_dropped(r#"{"filters": null}"#, "{}")]
#[case::unknown_is_dropped(r#"{"limit": 3}"#, "{}")]
fn search_filter_keeps_missing_zero_and_negative_values(
    #[case] input: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        requested_search_params(&params(input), &DefaultConfig),
        params(expected)
    );
}

#[test]
fn request_metadata_prefers_litellm_metadata_even_when_empty() {
    let kwargs = params(r#"{"litellm_metadata": {"team": "a"}, "metadata": {"team": "b"}}"#);
    assert_eq!(
        vector_store_request_metadata(&kwargs),
        params(r#"{"team": "a"}"#)
    );
    let empty = params(r#"{"litellm_metadata": {}, "metadata": {"team": "b"}}"#);
    assert_eq!(vector_store_request_metadata(&empty), Params::new());
}

#[rstest]
#[case::falls_back(
    r#"{"litellm_metadata": "raw", "metadata": {"team": "b"}}"#,
    r#"{"team": "b"}"#
)]
#[case::string_metadata(r#"{"metadata": "raw"}"#, "{}")]
#[case::wholly_absent("{}", "{}")]
#[case::null_slots(r#"{"litellm_metadata": null, "metadata": null}"#, "{}")]
fn request_metadata_falls_back_from_non_dictionaries(#[case] kwargs: &str, #[case] expected: &str) {
    assert_eq!(
        vector_store_request_metadata(&params(kwargs)),
        params(expected)
    );
}

#[test]
fn request_metadata_does_not_change_its_sources() {
    let kwargs = params(r#"{"litellm_metadata": {"team": "a"}}"#);
    let untouched = kwargs.clone();
    let _metadata = vector_store_request_metadata(&kwargs);
    assert_eq!(kwargs, untouched);
}

#[test]
fn metadata_entries_reject_a_non_string_key() {
    let error = metadata_map(
        vec![(Json::Number(Number::from(1_u8)), text("a"))],
        "litellm_metadata",
    )
    .expect_err("non-string key");
    assert_eq!(
        error,
        RequestMetadataError::NonStringKey {
            field: "litellm_metadata"
        }
    );
    assert_eq!(
        error.to_string(),
        "the litellm_metadata keys must be strings"
    );
}

#[test]
fn metadata_entries_build_a_string_keyed_map() {
    let built = metadata_map(vec![(text("team"), text("a"))], "metadata").expect("map");
    assert_eq!(built, params(r#"{"team": "a"}"#));
}

async fn provider(listener: TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.expect("connection");
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.expect("request head"));
    }
    let head = String::from_utf8(received).expect("head text");
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().expect("length"));
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.expect("request body");
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{EMBEDDING_ANSWER}",
        EMBEDDING_ANSWER.len()
    );
    stream.write_all(reply.as_bytes()).await.expect("reply");
    format!("{head}{}", String::from_utf8(body).expect("body text"))
}

fn request_body(sent: &str) -> Params {
    params(sent.split("\r\n\r\n").nth(1).expect("body"))
}

async fn listen() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}/v1", listener.local_addr().expect("address"));
    (listener, base)
}

fn direct_executor(base: &str) -> DirectEmbeddingExecutor {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&format!(
        "model_list:\n  - model_name: text-embedding-3-small\n    litellm_params: {{model: openai/text-embedding-3-small, api_base: '{base}', api_key: sk-test}}\n"
    ))
    .expect("config");
    let deployments: Vec<ChatDeployment> = config
        .model_list
        .iter()
        .map(|deployment| {
            ChatDeployment::resolve(deployment, &OpenAiDefaults::default()).expect("deployment")
        })
        .collect();
    let policy =
        Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("policy"));
    DirectEmbeddingExecutor {
        executor: ChatExecutor {
            client: ProxiedClient::outbound(&policy).expect("client"),
            deployments: deployments.clone(),
            catalog: CatalogCapabilities {
                chat_models: BTreeSet::new(),
                text_models: BTreeSet::new(),
            },
            random: Mutex::new(StdRng::seed_from_u64(0)),
        },
        deployments,
    }
}

#[tokio::test]
async fn direct_executor_posts_query_embedding_and_returns_the_mock_vector() {
    let (listener, base) = listen().await;
    let executor = direct_executor(&base);
    let configuration = params(r#"{"dimensions": 2}"#);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed("openai/text-embedding-3-small", "hello", &configuration)
    );
    assert!(sent.starts_with("POST /v1/embeddings HTTP/1.1\r\n"));
    assert!(sent.contains("authorization: Bearer sk-test"));
    assert_eq!(
        request_body(&sent),
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"], "dimensions": 2}"#)
    );
    let response: EmbeddingResponse = answered.expect("embedding");
    assert_eq!(response.object, "list");
    assert_eq!(response.model, "text-embedding-3-small");
    assert_eq!(response.data.first().map(|record| record.index), Some(0));
    assert_eq!(response.usage.prompt_tokens, 1);
    assert_eq!(response.usage.total_tokens, 1);
}

#[tokio::test]
async fn direct_executor_aembed_posts_the_same_request() {
    let (listener, base) = listen().await;
    let executor = direct_executor(&base);
    let configuration = params(r#"{"dimensions": 2}"#);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.aembed("openai/text-embedding-3-small", "hello", &configuration)
    );
    assert_eq!(
        request_body(&sent),
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"], "dimensions": 2}"#)
    );
    assert_eq!(answered.expect("embedding").object, "list");
}

#[rstest]
#[case::model("model")]
#[case::input("input")]
#[tokio::test]
async fn direct_executor_rejects_explicit_arguments_in_the_configuration(#[case] name: &str) {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let configuration = Params::from([(name.to_owned(), text("given"))]);
    let error = executor
        .embed("openai/text-embedding-3-small", "hello", &configuration)
        .await
        .expect_err("duplicate argument");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::DuplicateArgument { name: field } if field.as_str() == name
    ));
}

#[tokio::test]
async fn direct_executor_keeps_metadata_out_of_the_provider_body() {
    let (listener, base) = listen().await;
    let executor = direct_executor(&base);
    let configuration = params(r#"{"dimensions": 2, "metadata": {"keep": "yes"}}"#);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed("openai/text-embedding-3-small", "hello", &configuration)
    );
    let _answered = answered.expect("embedding");
    assert_eq!(
        request_body(&sent),
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"], "dimensions": 2}"#)
    );
}

#[tokio::test]
async fn direct_executor_rejects_an_unknown_configuration_parameter() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let error = executor
        .embed(
            "openai/text-embedding-3-small",
            "hello",
            &params(r#"{"input_type": "search"}"#),
        )
        .await
        .expect_err("unsupported parameter");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::UnsupportedParameter { name } if name == "input_type"
    ));
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouterCall {
    model: String,
    input: Vec<String>,
    configuration: Params,
    asynchronous: bool,
}

#[derive(Debug, Default)]
struct FakeRouter {
    resolved: Vec<String>,
    deployments: Vec<ModelDeployment>,
    calls: Mutex<Vec<RouterCall>>,
    teams: Mutex<Vec<Option<String>>>,
}

impl FakeRouter {
    fn with_resolved(model: &str) -> Self {
        Self {
            resolved: vec![model.to_owned()],
            deployments: Vec::new(),
            calls: Mutex::new(Vec::new()),
            teams: Mutex::new(Vec::new()),
        }
    }

    fn with_deployment(entries: &str) -> Self {
        let config = ConfigLoader {
            lookup: BTreeMap::new(),
        }
        .load(entries)
        .expect("model list");
        Self {
            resolved: Vec::new(),
            deployments: config.model_list,
            calls: Mutex::new(Vec::new()),
            teams: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<RouterCall> {
        self.calls.lock().expect("calls").clone()
    }

    fn teams(&self) -> Vec<Option<String>> {
        self.teams.lock().expect("teams").clone()
    }
}

fn answer() -> EmbeddingResponse {
    serde_json::from_str(EMBEDDING_ANSWER).expect("embedding answer")
}

#[async_trait::async_trait]
impl VectorStoreRouter for FakeRouter {
    fn resolved_litellm_models(&self, _model: &str, team_id: Option<&str>) -> Vec<String> {
        self.teams
            .lock()
            .expect("teams")
            .push(team_id.map(str::to_owned));
        self.resolved.clone()
    }

    fn model_list(&self) -> Vec<ModelDeployment> {
        self.deployments.clone()
    }

    async fn embedding(
        &self,
        model: &str,
        input: &[String],
        configuration: Params,
    ) -> Result<EmbeddingResponse, BaseLlmException> {
        self.calls.lock().expect("calls").push(RouterCall {
            model: model.to_owned(),
            input: input.to_vec(),
            configuration,
            asynchronous: false,
        });
        Ok(answer())
    }

    async fn aembedding(
        &self,
        model: &str,
        input: &[String],
        configuration: Params,
    ) -> Result<EmbeddingResponse, BaseLlmException> {
        self.calls.lock().expect("calls").push(RouterCall {
            model: model.to_owned(),
            input: input.to_vec(),
            configuration,
            asynchronous: true,
        });
        Ok(answer())
    }
}

fn router(router: FakeRouter) -> RouterEmbeddingExecutor<FakeRouter> {
    RouterEmbeddingExecutor {
        router,
        metadata: params(r#"{"team": "new", "user_api_key_team_id": "t"}"#),
        direct: direct_executor("http://127.0.0.1:9/v1"),
    }
}

#[tokio::test]
async fn router_is_selected_when_a_model_resolves() {
    let executor = router(FakeRouter::with_resolved("openai/text-embedding-3-small"));
    let response = executor
        .embed(
            "openai/text-embedding-3-small",
            "hello",
            &params(r#"{"model": "ignored", "input": "ignored", "metadata": {"keep": "yes", "team": "old"}, "dimensions": 2}"#),
        )
        .await
        .expect("router embedding");
    assert_eq!(response.object, "list");
    assert_eq!(
        executor.router.calls(),
        vec![RouterCall {
            model: String::from("openai/text-embedding-3-small"),
            input: vec![String::from("hello")],
            configuration: params(
                r#"{"dimensions": 2, "metadata": {"keep": "yes", "team": "new", "user_api_key_team_id": "t"}}"#
            ),
            asynchronous: false,
        }]
    );
    assert_eq!(executor.router.teams(), vec![Some(String::from("t"))]);
}

#[tokio::test]
async fn router_is_selected_when_a_deployment_serves_the_model() {
    let executor = router(FakeRouter::with_deployment(
        "model_list:\n  - model_name: alias\n    litellm_params: {model: openai/text-embedding-3-small}\n",
    ));
    let response = executor
        .aembed(
            "openai/text-embedding-3-small",
            "hello",
            &params(r#"{"dimensions": 2}"#),
        )
        .await
        .expect("router embedding");
    assert_eq!(response.object, "list");
    let calls = executor.router.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls.first().is_some_and(|call| call.asynchronous));
}

#[tokio::test]
async fn direct_execution_is_selected_without_a_resolved_model_or_deployment() {
    let (listener, base) = listen().await;
    let mut executor = router(FakeRouter::default());
    executor.direct = direct_executor(&base);
    let configuration = params("{}");
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed("openai/text-embedding-3-small", "hello", &configuration)
    );
    assert_eq!(
        request_body(&sent),
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"]}"#)
    );
    assert_eq!(answered.expect("embedding").object, "list");
    assert!(executor.router.calls().is_empty());
}

#[rstest]
#[case::string_team(r#"{"user_api_key_team_id": "t"}"#, Some("t"))]
#[case::number_team(r#"{"user_api_key_team_id": 7}"#, None)]
#[case::no_team("{}", None)]
#[tokio::test]
async fn non_string_team_ids_resolve_without_a_team(
    #[case] metadata: &str,
    #[case] expected: Option<&str>,
) {
    let executor = RouterEmbeddingExecutor {
        router: FakeRouter::with_resolved("openai/text-embedding-3-small"),
        metadata: params(metadata),
        direct: direct_executor("http://127.0.0.1:9/v1"),
    };
    executor
        .embed("openai/text-embedding-3-small", "hello", &params("{}"))
        .await
        .expect("router embedding");
    assert_eq!(executor.router.teams(), vec![expected.map(str::to_owned)]);
}

#[tokio::test]
async fn configured_metadata_that_is_not_a_map_is_ignored() {
    let executor = router(FakeRouter::with_resolved("openai/text-embedding-3-small"));
    executor
        .embed(
            "openai/text-embedding-3-small",
            "hello",
            &params(
                r#"{"metadata": "raw", "dimensions": 2, "model": "ignored", "input": "ignored"}"#,
            ),
        )
        .await
        .expect("router embedding");
    assert_eq!(
        executor
            .router
            .calls()
            .first()
            .map(|call| call.configuration.clone()),
        Some(params(
            r#"{"dimensions": 2, "metadata": {"team": "new", "user_api_key_team_id": "t"}}"#
        ))
    );
}
