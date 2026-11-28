use rstest::rstest;

use super::create::{create_call, run_create};
use super::*;
use crate::api::vector_store_http_write::service::LogContext;
use crate::api::vector_store_search::mock;
use crate::api::vector_store_search::service::{SearchCall, search_call::search};

pub(super) async fn run_search<S>(
    logs: &mut Logs,
    deps: &SdkDeps<'_, S>,
    call: &SearchCall<'_>,
) -> Result<Json, VectorStoreSearchError>
where
    S: ClientSource<Client: WriteTransport>,
{
    let env = pre_post_env(&logs.redactor, &logs.sink, logs.now);
    search(
        deps,
        call,
        &mut LogContext {
            state: &mut logs.state,
            env: &env,
        },
    )
    .await
}

fn search_call<'call>(
    query: &'call SearchQuery,
    optional: &'call Params,
    kwargs: &'call Params,
    entry: Entry,
) -> SearchCall<'call> {
    SearchCall {
        vector_store_id: "vs_a",
        query,
        optional,
        kwargs,
        extra_headers: None,
        extra_body: None,
        timeout: None,
        custom_llm_provider: None,
        executor: None,
        entry,
    }
}

#[tokio::test]
async fn search_optional_fields() {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let optional = params(SEARCH_OPTIONALS);
    let kwargs = params(
        r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1","litellm_call_id":null}"#,
    );
    let query = SearchQuery::Sequence(vec![String::from("one"), String::from("two")]);
    let call = search_call(&query, &optional, &kwargs, Entry::Asynchronous);
    let deps = fakes.deps(&source, None);
    let result = run_search(&mut logs, &deps, &call).await;
    assert_eq!(
        result.expect("the mock provider answers"),
        json(SEARCH_RESULT)
    );
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.search_optionals.last().expect("the transform ran"),
            &params(SEARCH_OPTIONALS)
        );
        assert_eq!(seen.search_timeouts, vec![Some(Duration::from_mins(100))]);
    });
}

#[tokio::test]
async fn search_logs_optional_params() {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let optional = params(SEARCH_OPTIONALS);
    let kwargs = params(
        r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1","litellm_call_id":null}"#,
    );
    let query = SearchQuery::Sequence(vec![String::from("one"), String::from("two")]);
    let call = search_call(&query, &optional, &kwargs, Entry::Asynchronous);
    let deps = fakes.deps(&source, None);
    run_search(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    assert_eq!(
        logs.detail("optional_params"),
        json(
            r#"{"vector_store_id":"vs_a","query":["one","two"],"filters":{"type":"eq","key":"tag","value":"x"},"max_num_results":0,"ranking_options":{"ranker":"auto"},"rewrite_query":false}"#
        )
    );
    assert_eq!(
        logs.detail("custom_llm_provider"),
        Json::Text(String::from("openai"))
    );
    assert_eq!(
        logs.detail("model"),
        Json::Text(String::from("vector-store"))
    );
}

#[tokio::test]
async fn search_logs_litellm_params() {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let optional = Params::new();
    let kwargs = params(
        r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1","litellm_call_id":null}"#,
    );
    let query = SearchQuery::Text(String::from("one"));
    let call = search_call(&query, &optional, &kwargs, Entry::Sync);
    let deps = fakes.deps(&source, None);
    run_search(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    let Json::Object(litellm_params) = logs.detail("litellm_params") else {
        panic!("the logged litellm params are an object");
    };
    assert_eq!(litellm_params.get("litellm_call_id"), Some(&Json::Null));
    assert_eq!(
        litellm_params.get("vector_store_id"),
        Some(&Json::Text(String::from("vs_a")))
    );
    assert_eq!(
        litellm_params.get("use_litellm_proxy"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        litellm_params.get("api_base"),
        Some(&Json::Text(String::from("https://mock.invalid/v1")))
    );
}

#[rstest]
#[case::unset(None, Duration::from_mins(100))]
#[case::zero(Some(0.0_f64), Duration::from_mins(100))]
#[case::given(Some(3.0_f64), Duration::from_secs(3))]
#[tokio::test]
async fn search_timeout_follows_request_timeout(
    #[case] given: Option<f64>,
    #[case] expected: Duration,
) {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let optional = Params::new();
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
    let query = SearchQuery::Text(String::from("one"));
    let mut call = search_call(&query, &optional, &kwargs, Entry::Sync);
    call.timeout = given;
    let deps = fakes.deps(&source, None);
    run_search(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    fakes
        .seen
        .with(|seen| assert_eq!(seen.search_timeouts, vec![Some(expected)]));
}

#[tokio::test]
async fn registry_overlays_search_credentials() {
    let mut fakes = Fakes::new();
    let credentials = JsonObject::from([(
        String::from("api_key"),
        Json::Text(String::from("sk-registry")),
    )]);
    fakes.registry = VectorStoreRegistry {
        stores: vec![managed_store(Some("store-credential"))],
        clock: Arc::new(Utc::now),
        credentials: Arc::new(FixedCredentials(credentials)),
    };
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let query = SearchQuery::Text(String::from("one"));
    let kwargs = params(
        r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1","litellm_call_id":null}"#,
    );
    let optional = params(r#"{"name":"notes"}"#);
    let call = search_call(&query, &optional, &kwargs, Entry::Sync);
    let deps = fakes.deps(&source, None);
    run_search(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    // the overlay lands before the pre-call logging, so the recorded params carry it too
    let Json::Object(litellm_params) = logs.detail("litellm_params") else {
        panic!("the logged litellm params are an object");
    };
    assert_eq!(
        litellm_params.get("api_key"),
        Some(&Json::Text(String::from("sk-registry")))
    );
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.search_litellm_params
                .last()
                .expect("the transform ran")
                .get("api_key"),
            Some(&Json::Text(String::from("sk-registry")))
        );
    });
}

#[tokio::test]
async fn create_never_reads_the_registry() {
    let mut fakes = Fakes::new();
    let credentials = JsonObject::from([(
        String::from("api_key"),
        Json::Text(String::from("sk-registry")),
    )]);
    fakes.registry = VectorStoreRegistry {
        stores: vec![managed_store(Some("store-credential"))],
        clock: Arc::new(Utc::now),
        credentials: Arc::new(FixedCredentials(credentials)),
    };
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = params(r#"{"name":"notes"}"#);
    let kwargs = params(r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1"}"#);
    let invocation = create_call(&optional, &kwargs, Entry::Sync);
    let deps = fakes.deps(&source, None);
    run_create(&mut logs, &deps, &invocation)
        .await
        .expect("the mock provider answers");
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.create_api_keys.last(),
            Some(&Some(String::from("sk-client")))
        );
    });
}

#[tokio::test]
async fn invalid_executor_fails_before_any_http() {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    // a mock that would answer does not run: the executor is checked before the mock
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1","mock_response":"yes"}"#);
    let optional = Params::new();
    let query = SearchQuery::Text(String::from("one"));
    let mut call = search_call(&query, &optional, &kwargs, Entry::Sync);
    call.executor = Some(DirectExecutor::Invalid);
    let deps = fakes.deps(&source, None);
    let error = run_search(&mut logs, &deps, &call)
        .await
        .expect_err("a non-executor is refused");
    assert_eq!(
        error.to_string(),
        "Invalid direct vector store embedding executor"
    );
    fakes.seen.with(|seen| assert_eq!(seen.clients.len(), 0));
}

#[tokio::test]
async fn supplied_executor_reaches_the_embedding_source() {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let executor = RecordingExecutor;
    let mut logs = Logs::new("vector_store_search");
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
    let optional = Params::new();
    let query = SearchQuery::Text(String::from("one"));
    let mut call = search_call(&query, &optional, &kwargs, Entry::Sync);
    call.executor = Some(DirectExecutor::Executor(&executor));
    let deps = fakes.deps(&source, None);
    run_search(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    fakes
        .seen
        .with(|seen| assert_eq!(seen.executor_supplied, vec![true]));
}

// B1059 reads the request metadata from the kwargs after the private executor argument is popped and before asearch or the registry
#[tokio::test]
async fn executor_sees_the_request_kwargs_before_the_registry() {
    let mut fakes = Fakes::new();
    let credentials = JsonObject::from([(
        String::from("api_key"),
        Json::Text(String::from("sk-registry")),
    )]);
    fakes.registry = VectorStoreRegistry {
        stores: vec![managed_store(Some("store-credential"))],
        clock: Arc::new(Utc::now),
        credentials: Arc::new(FixedCredentials(credentials)),
    };
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let kwargs = params(
        r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1","asearch":true,"_direct_vector_store_embedding_executor":{}}"#,
    );
    let optional = Params::new();
    let query = SearchQuery::Text(String::from("one"));
    let call = search_call(&query, &optional, &kwargs, Entry::Asynchronous);
    let deps = fakes.deps(&source, None);
    run_search(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    fakes.seen.with(|seen| {
        let recorded = seen
            .executor_kwargs
            .last()
            .expect("the executor source ran");
        assert_eq!(
            recorded.get("api_key"),
            Some(&Json::Text(String::from("sk-client")))
        );
        assert_eq!(recorded.get("asearch"), Some(&Json::Bool(true)));
        assert_eq!(
            recorded.get("_direct_vector_store_embedding_executor"),
            None
        );
    });
}

// the mock answers before the provider is resolved, so a provider no enum member names still returns the sample
#[tokio::test]
async fn search_mock_precedes_provider_resolution() {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let optional = Params::new();
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1","mock_response":"yes"}"#);
    let query = SearchQuery::Text(String::from("one"));
    let mut call = search_call(&query, &optional, &kwargs, Entry::Sync);
    call.custom_llm_provider = Some("bogus");
    let deps = fakes.deps(&source, None);
    let result = run_search(&mut logs, &deps, &call)
        .await
        .expect("the mock answers");
    assert_eq!(result, json(SAMPLE_SEARCH));
    fakes.seen.with(|seen| assert_eq!(seen.clients.len(), 0));
}

#[rstest]
#[case::sample_string(r#"{"mock_response":"yes"}"#, Some(SAMPLE_SEARCH))]
#[case::list_replaces_data(
    r#"{"mock_response":[{"score":0.5,"content":[{"text":"hit","type":"text"}]}]}"#,
    Some(
        r#"{"object":"vector_store.search_results.page","search_query":"sample query","data":[{"score":0.5,"content":[{"text":"hit","type":"text"}]}]}"#
    )
)]
#[case::empty_string(r#"{"mock_response":""}"#, None)]
#[case::empty_list(r#"{"mock_response":[]}"#, None)]
#[case::object_proceeds(r#"{"mock_response":{"anything":"goes"}}"#, None)]
#[tokio::test]
async fn vector_mock_truthiness(#[case] mock: &str, #[case] expected: Option<&str>) {
    let fakes = Fakes::new();
    let source = fakes.source(SEARCH_RESULT);
    let mut logs = Logs::new("vector_store_search");
    let query = SearchQuery::Text(String::from("one"));
    let optional = Params::new();
    let kwargs = params(mock);
    let call = search_call(&query, &optional, &kwargs, Entry::Sync);
    let deps = fakes.deps(&source, None);
    let result = run_search(&mut logs, &deps, &call)
        .await
        .expect("the search answers");
    match expected {
        Some(text) => assert_eq!(result, json(text)),
        None => assert_eq!(result, json(SEARCH_RESULT)),
    }
}

#[test]
fn mock_samples() {
    assert_eq!(mock::mock_search_response(None), json(SAMPLE_SEARCH));
    assert_eq!(
        mock::mock_search_response(Some(&Json::List(Vec::new()))),
        json(SEARCH_RESULT)
    );
    assert_eq!(
        mock::mock_create_response(None),
        json(
            r#"{"id":"vs_mock123","object":"vector_store","created_at":1699061776,"name":"Mock Vector Store","bytes":0,"file_counts":{"in_progress":0,"completed":0,"failed":0,"cancelled":0,"total":0},"status":"completed","expires_after":null,"expires_at":null,"last_active_at":null,"metadata":null}"#
        )
    );
}

#[tokio::test]
async fn supplied_client_reaches_the_transport() {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let supplied = FakeClient {
        name: String::from("supplied"),
        answer: (200, HeaderMap::new(), SEARCH_RESULT.as_bytes().to_vec()),
        seen: Shared::default(),
    };
    let mut logs = Logs::new("vector_store_search");
    let optional = Params::new();
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
    let query = SearchQuery::Text(String::from("one"));
    let call = search_call(&query, &optional, &kwargs, Entry::Asynchronous);
    let deps = fakes.deps(
        &source,
        Some(SuppliedClient {
            kind: ClientKind::Asynchronous,
            transport: &supplied,
        }),
    );
    run_search(&mut logs, &deps, &call)
        .await
        .expect("the supplied client answers");
    supplied
        .seen
        .with(|seen| assert_eq!(seen.clients, vec![String::from("supplied")]));
    fakes
        .seen
        .with(|seen| assert_eq!(seen.clients, Vec::<String>::new()));
}

pub(super) struct SearchConfigFake {
    pub(super) seen: Shared,
}

#[async_trait]
impl SearchConfig for SearchConfigFake {
    fn validate_environment(
        &self,
        headers: OpenAiHeaders,
        _litellm_params: &Params,
    ) -> Result<OpenAiHeaders, SearchError> {
        Ok(headers)
    }

    fn complete_url(&self, litellm_params: &Params) -> Result<String, SearchError> {
        match litellm_params.get("api_base") {
            Some(Json::Text(base)) => Ok(base.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => Ok(String::from("https://mock.invalid/v1")),
        }
    }

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<(String, Params), SearchError> {
        self.seen.with(|seen| {
            seen.search_litellm_params
                .push(request.litellm_params.clone());
            seen.search_optionals.push(request.optional_params.clone());
        });
        Ok((
            String::from("https://mock.invalid/v1/vector_stores/vs_a/search"),
            request.litellm_params.clone(),
        ))
    }

    fn transform_search_response(
        &self,
        response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, SearchError> {
        serde_json::from_slice(&response.body)
            .map_err(|source| {
                self.error_class(
                    &source.to_string(),
                    response.status_code,
                    WireHeaders::new(),
                )
            })
            .map_err(SearchError::Provider)
    }

    fn error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: WireHeaders,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}
