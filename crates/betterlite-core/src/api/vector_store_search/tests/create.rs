use rstest::rstest;

use super::*;
use crate::api::vector_store_http_write::service::LogContext;
use crate::api::vector_store_search::service::{CreateCall, create_call::create};

pub(super) async fn run_create<S>(
    logs: &mut Logs,
    deps: &SdkDeps<'_, S>,
    call: &CreateCall<'_>,
) -> Result<Json, VectorStoreSearchError>
where
    S: ClientSource<Client: WriteTransport>,
{
    let env = pre_post_env(&logs.redactor, &logs.sink, logs.now);
    create(
        deps,
        call,
        &mut LogContext {
            state: &mut logs.state,
            env: &env,
        },
    )
    .await
}

pub(super) fn create_call<'call>(
    optional: &'call Params,
    kwargs: &'call Params,
    entry: Entry,
) -> CreateCall<'call> {
    CreateCall {
        optional,
        kwargs,
        extra_headers: None,
        extra_body: None,
        timeout: None,
        custom_llm_provider: None,
        entry,
    }
}

#[rstest]
#[case::sync(Entry::Sync)]
#[case::asynchronous(Entry::Asynchronous)]
#[tokio::test]
async fn create_optional_fields(#[case] entry: Entry) {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = params(CREATE_OPTIONALS);
    let kwargs = params(r#"{"api_key":"sk-client","api_base":"https://mock.invalid/v1"}"#);
    let call = create_call(&optional, &kwargs, entry);
    let deps = fakes.deps(&source, None);
    let result = run_create(&mut logs, &deps, &call).await;
    assert_eq!(
        result.expect("the mock provider answers"),
        json(CREATE_RESULT)
    );
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.bodies.last().expect("the create posted"),
            &json(CREATE_OPTIONALS)
        );
        assert_eq!(seen.create_api_keys, vec![Some(String::from("sk-client"))]);
        assert_eq!(seen.ssl_verify.last(), Some(&None));
    });
    assert_eq!(logs.detail("optional_params"), json(CREATE_OPTIONALS));
    assert_eq!(
        logs.detail("custom_llm_provider"),
        Json::Text(String::from("openai"))
    );
    assert_eq!(
        logs.detail("model"),
        Json::Text(String::from("vector-store"))
    );
    let Json::Object(litellm_params) = logs.detail("litellm_params") else {
        panic!("the logged litellm params are an object");
    };
    assert_eq!(litellm_params.get("litellm_call_id"), Some(&Json::Null));
}

// an empty create dict proceeds to HTTP, and a nonempty one comes back unchanged
#[rstest]
#[case::empty_posted(r#"{"api_base":"https://mock.invalid/v1","mock_response":{}}"#, true)]
#[case::nonempty_returned(
    r#"{"api_base":"https://mock.invalid/v1","mock_response":{"id":"vs_a","object":"vector_store"}}"#,
    false
)]
#[tokio::test]
async fn vector_mock_create_truthiness(#[case] mock: &str, #[case] posted: bool) {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = Params::new();
    let kwargs = params(mock);
    let call = create_call(&optional, &kwargs, Entry::Sync);
    let deps = fakes.deps(&source, None);
    let before = fakes.seen.with(|seen| seen.clients.len());
    let result = run_create(&mut logs, &deps, &call)
        .await
        .expect("the create answers");
    assert_eq!(result, json(CREATE_RESULT));
    let after = fakes.seen.with(|seen| seen.clients.len());
    assert_eq!(after, before.saturating_add(usize::from(posted)));
}

// a create with no optionals sends the five declared fields as nulls
#[tokio::test]
async fn create_sends_the_declared_fields() {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = Params::new();
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
    let mut headers = BTreeMap::new();
    headers.insert("X-Caller".to_owned(), "yes".to_owned());
    let mut call = create_call(&optional, &kwargs, Entry::Sync);
    call.extra_headers = Some(&headers);
    let deps = fakes.deps(&source, None);
    run_create(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    fakes.seen.with(|seen| {
        assert_eq!(
            seen.bodies.last().expect("the create posted"),
            &json(
                r#"{"name":null,"file_ids":null,"expires_after":null,"chunking_strategy":null,"metadata":null}"#
            )
        );
        let sent = seen.requests.last().expect("the request was recorded");
        assert_eq!(
            sent.headers
                .get("X-Caller")
                .map(|value| value.to_str().expect("the header is text")),
            Some("yes")
        );
    });
}

// the mock answers before the provider is resolved, so a provider no enum member names still returns it
#[tokio::test]
async fn create_mock_precedes_provider_resolution() {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = Params::new();
    let kwargs = params(
        r#"{"api_base":"https://mock.invalid/v1","mock_response":{"id":"vs_a","object":"vector_store"}}"#,
    );
    let mut call = create_call(&optional, &kwargs, Entry::Sync);
    call.custom_llm_provider = Some("bogus");
    let deps = fakes.deps(&source, None);
    let result = run_create(&mut logs, &deps, &call)
        .await
        .expect("the mock answers");
    assert_eq!(result, json(CREATE_RESULT));
    fakes.seen.with(|seen| assert_eq!(seen.clients.len(), 0));
}

// ssl_verify is the GenericLiteLLMParams flag the caller set, handed to the client source as given
#[rstest]
#[case::verify(r#"{"api_base":"https://mock.invalid/v1","ssl_verify":true}"#, true)]
#[case::skip(r#"{"api_base":"https://mock.invalid/v1","ssl_verify":false}"#, false)]
#[tokio::test]
async fn create_ssl_verify_reaches_the_client(#[case] input: &str, #[case] expected: bool) {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = Params::new();
    let kwargs = params(input);
    let call = create_call(&optional, &kwargs, Entry::Sync);
    let deps = fakes.deps(&source, None);
    run_create(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    fakes
        .seen
        .with(|seen| assert_eq!(seen.ssl_verify, vec![Some(expected)]));
}

// an asynchronous create waits the resolved timeout: null uses REQUEST_TIMEOUT and an explicit value is that many seconds
#[rstest]
#[case::unset(None, Duration::from_mins(100))]
#[case::given(Some(3.0_f64), Duration::from_secs(3))]
#[tokio::test]
async fn create_timeout_follows_request_timeout(
    #[case] given: Option<f64>,
    #[case] expected: Duration,
) {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = Params::new();
    let kwargs = params(r#"{"api_base":"https://mock.invalid/v1"}"#);
    let mut call = create_call(&optional, &kwargs, Entry::Asynchronous);
    call.timeout = given;
    let deps = fakes.deps(&source, None);
    run_create(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    fakes
        .seen
        .with(|seen| assert_eq!(seen.search_timeouts, vec![Some(expected)]));
}

// update_from_kwargs(kwargs=kwargs) merges the caller's litellm_metadata into the logged litellm_params
#[tokio::test]
async fn create_logs_kwargs_metadata() {
    let fakes = Fakes::new();
    let source = fakes.source(CREATE_RESULT);
    let mut logs = Logs::new("vector_store_create");
    let optional = Params::new();
    let kwargs =
        params(r#"{"api_base":"https://mock.invalid/v1","litellm_metadata":{"trace":"caller"}}"#);
    let call = create_call(&optional, &kwargs, Entry::Sync);
    let deps = fakes.deps(&source, None);
    run_create(&mut logs, &deps, &call)
        .await
        .expect("the mock provider answers");
    let Json::Object(litellm_params) = logs.detail("litellm_params") else {
        panic!("the logged litellm params are an object");
    };
    assert_eq!(
        litellm_params.get("litellm_metadata"),
        Some(&json(r#"{"trace":"caller"}"#))
    );
    assert_eq!(
        litellm_params.get("metadata"),
        Some(&json(r#"{"trace":"caller"}"#))
    );
}
