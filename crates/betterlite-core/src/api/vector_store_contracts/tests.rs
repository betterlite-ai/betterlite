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
use secrecy::SecretString;
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

fn executor_over(
    deployments: Vec<ChatDeployment>,
    defaults: OpenAiDefaults,
) -> DirectEmbeddingExecutor {
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
        defaults,
    }
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
    executor_over(deployments, OpenAiDefaults::default())
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

async fn embed_through(executor: &DirectEmbeddingExecutor, given: &str) -> (String, Params) {
    let (listener, base) = listen().await;
    let mut configuration = params(given);
    configuration
        .entry("api_base".to_owned())
        .or_insert_with(|| text(&base));
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed("openai/text-embedding-3-small", "hello", &configuration)
    );
    let _answered = answered.expect("embedding");
    let body = request_body(&sent);
    (sent, body)
}

#[tokio::test]
async fn direct_executor_sends_unknown_parameters_in_the_provider_body() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let (_sent, body) = embed_through(&executor, r#"{"input_type": "search"}"#).await;
    assert_eq!(
        body,
        params(
            r#"{"model": "text-embedding-3-small", "input": ["hello"], "input_type": "search"}"#
        )
    );
}

#[tokio::test]
async fn direct_executor_merges_extra_body_into_the_provider_body() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let (_sent, body) =
        embed_through(&executor, r#"{"extra_body": {"foo": 1}, "user": "u1"}"#).await;
    assert_eq!(
        body,
        params(
            r#"{"model": "text-embedding-3-small", "input": ["hello"], "user": "u1", "foo": 1}"#
        )
    );
}

#[tokio::test]
async fn direct_executor_accepts_litellm_call_parameters_without_sending_them() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let configuration = r#"{"max_retries": 2, "num_retries": 3, "caching": true, "drop_params": true, "api_version": "v1", "api_type": "openai", "custom_llm_provider": "openai", "litellm_call_id": "c", "logger_fn": "f", "extra_headers": {"x-extra": "1"}, "headers": {"x-header": "2"}}"#;
    let (_sent, body) = embed_through(&executor, configuration).await;
    assert_eq!(
        body,
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"]}"#)
    );
}

#[tokio::test]
async fn direct_executor_forwards_encoding_format_and_user() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let (_sent, body) =
        embed_through(&executor, r#"{"encoding_format": "float", "user": "u1"}"#).await;
    assert_eq!(
        body,
        params(
            r#"{"model": "text-embedding-3-small", "input": ["hello"], "encoding_format": "float", "user": "u1"}"#
        )
    );
}

#[tokio::test]
async fn direct_executor_calls_a_model_no_deployment_serves() {
    let executor = executor_over(Vec::new(), OpenAiDefaults::default());
    let (listener, base) = listen().await;
    let configuration = Params::from([
        ("api_base".to_owned(), text(&base)),
        ("api_key".to_owned(), text("sk-config")),
    ]);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed("openai/some-new-model", "hello", &configuration)
    );
    let _answered = answered.expect("embedding");
    assert!(sent.contains("authorization: Bearer sk-config"));
    assert_eq!(
        request_body(&sent),
        params(r#"{"model": "some-new-model", "input": ["hello"]}"#)
    );
}

#[tokio::test]
async fn direct_executor_falls_back_to_the_environment_defaults_without_a_deployment() {
    let (listener, base) = listen().await;
    let executor = executor_over(
        Vec::new(),
        OpenAiDefaults {
            api_bases: vec![base],
            api_keys: vec![SecretString::from("sk-default")],
            request_timeout: None,
            organization: None,
        },
    );
    let configuration = Params::new();
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed("openai/some-new-model", "hello", &configuration)
    );
    let _answered = answered.expect("embedding");
    assert!(sent.contains("authorization: Bearer sk-default"));
}

#[tokio::test]
async fn direct_executor_keeps_the_deployment_when_options_are_null() {
    let (listener, base) = listen().await;
    let executor = direct_executor(&base);
    let configuration = params(r#"{"api_base": null, "api_key": null, "timeout": null}"#);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed("openai/text-embedding-3-small", "hello", &configuration)
    );
    let _answered = answered.expect("embedding");
    assert!(sent.contains("authorization: Bearer sk-test"));
}

#[tokio::test]
async fn direct_executor_overrides_the_deployment_base_and_key() {
    let (listener, base) = listen().await;
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let configuration = Params::from([
        ("api_base".to_owned(), text(&base)),
        ("api_key".to_owned(), text("sk-override")),
    ]);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed("openai/text-embedding-3-small", "hello", &configuration)
    );
    let _answered = answered.expect("embedding");
    assert!(sent.starts_with("POST /v1/embeddings HTTP/1.1\r\n"));
    assert!(sent.contains("authorization: Bearer sk-override"));
}

#[tokio::test]
async fn direct_executor_gives_up_after_the_configured_timeout() {
    let (listener, base) = listen().await;
    let silent = async {
        let (mut stream, _) = listener.accept().await.expect("connection");
        let mut received = Vec::new();
        drop(stream.read_to_end(&mut received).await);
    };
    let executor = direct_executor(&base);
    let configuration = params(r#"{"timeout": 0.2}"#);
    let ((), outcome) = tokio::join!(
        silent,
        executor.embed("openai/text-embedding-3-small", "hello", &configuration)
    );
    assert!(matches!(
        outcome,
        Err(VectorStoreEmbeddingError::Embedding { .. })
    ));
}

#[rstest]
#[case::base("api_base", "1", "api_base", "string")]
#[case::key("api_key", "[]", "api_key", "string")]
#[case::timeout("timeout", r#""soon""#, "timeout", "number")]
#[case::extra_body("extra_body", r#""raw""#, "extra_body", "object")]
#[tokio::test]
async fn direct_executor_names_the_field_of_the_wrong_type(
    #[case] name: &str,
    #[case] value: &str,
    #[case] field: &str,
    #[case] expected: &str,
) {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let error = executor
        .embed(
            "openai/text-embedding-3-small",
            "hello",
            &params(&format!(r#"{{"{name}": {value}}}"#)),
        )
        .await
        .expect_err("wrong type");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::FieldType { field: found, expected: wanted }
            if found == field && wanted == expected
    ));
}

#[tokio::test]
async fn direct_executor_rejects_a_timeout_no_wait_can_last() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let error = executor
        .embed(
            "openai/text-embedding-3-small",
            "hello",
            &params(r#"{"timeout": -1}"#),
        )
        .await
        .expect_err("timeout");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::Timeout { seconds, .. } if seconds.is_sign_negative()
    ));
}

#[tokio::test]
async fn direct_executor_names_a_typed_parameter_of_the_wrong_type() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let error = executor
        .embed(
            "openai/text-embedding-3-small",
            "hello",
            &params(r#"{"dimensions": "two"}"#),
        )
        .await
        .expect_err("request");
    assert!(matches!(error, VectorStoreEmbeddingError::Request { .. }));
}

#[tokio::test]
async fn direct_executor_reports_defaults_that_make_no_deployment() {
    let executor = executor_over(
        Vec::new(),
        OpenAiDefaults {
            api_bases: vec![String::from("not a url")],
            api_keys: Vec::new(),
            request_timeout: None,
            organization: None,
        },
    );
    let error = executor
        .embed("openai/some-new-model", "hello", &Params::new())
        .await
        .expect_err("deployment");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::Deployment { .. }
    ));
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouterCall {
    entry: &'static str,
    model: String,
    input: Vec<String>,
    configuration: Params,
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

    fn record(
        &self,
        entry: &'static str,
        model: &str,
        input: &[String],
        configuration: Params,
    ) -> EmbeddingResponse {
        self.calls.lock().expect("calls").push(RouterCall {
            entry,
            model: model.to_owned(),
            input: input.to_vec(),
            configuration,
        });
        answer()
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
        Ok(self.record("embedding", model, input, configuration))
    }

    async fn aembedding(
        &self,
        model: &str,
        input: &[String],
        configuration: Params,
    ) -> Result<EmbeddingResponse, BaseLlmException> {
        Ok(self.record("aembedding", model, input, configuration))
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
            entry: "embedding",
            model: String::from("openai/text-embedding-3-small"),
            input: vec![String::from("hello")],
            configuration: params(
                r#"{"dimensions": 2, "metadata": {"keep": "yes", "team": "new", "user_api_key_team_id": "t"}}"#
            ),
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
    assert_eq!(
        calls.first().map(|call| call.model.as_str()),
        Some("openai/text-embedding-3-small")
    );
    assert_eq!(calls.first().map(|call| call.entry), Some("aembedding"));
}

#[rstest]
#[case::sync(false, "embedding")]
#[case::asynchronous(true, "aembedding")]
#[tokio::test]
async fn router_entry_point_follows_the_called_method(
    #[case] asynchronous: bool,
    #[case] expected: &str,
) {
    let executor = router(FakeRouter::with_resolved("openai/text-embedding-3-small"));
    if asynchronous {
        executor
            .aembed("openai/text-embedding-3-small", "hello", &params("{}"))
            .await
            .expect("router embedding");
    } else {
        executor
            .embed("openai/text-embedding-3-small", "hello", &params("{}"))
            .await
            .expect("router embedding");
    }
    let calls = executor.router.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls.first().map(|call| call.entry), Some(expected));
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
