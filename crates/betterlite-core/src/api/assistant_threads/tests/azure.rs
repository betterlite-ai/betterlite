#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::client_kind::ClientKind;
use betterlite_models::decisions::Json;
use betterlite_openai::assistants::client::SdkClient;
use rstest::rstest;
use secrecy::SecretString;

use super::super::target::{Operation, PrepareRequest, PreparedThread, Target};
use super::super::{
    FacadeCall, GetThreadCall, SuppliedClient, ThreadFacade as _, ThreadOperation, execute,
};
use super::{
    Capture, Harness, Recorder, THREAD, UNTYPED_404, assert_sdk_failure, create_call, empty_create,
    json, kwargs_of, message, secret_text,
};

fn azure_prepare<'call>(
    harness: &'call Harness,
    kwargs: &'call BTreeMap<String, Json>,
    client: Option<SuppliedClient<'call, Recorder>>,
) -> PreparedThread<'call, Recorder> {
    harness
        .threads()
        .prepare(PrepareRequest {
            provider: String::from("azure"),
            operation: Operation::Retrieve(String::from("thread_a")),
            client,
            kwargs,
            facade: ThreadOperation::Get,
            flag: None,
        })
        .expect("the call prepares")
}

#[test]
fn azure_gets_litellm_params_and_the_shared_settings() {
    let mut harness = Harness::new();
    harness.resources.azure_api_base = Some(String::from("https://azure.test"));
    harness.resources.azure_api_version = Some(String::from("2024-02-01"));
    harness.resources.azure_openai_api_key = Some(SecretString::from("azure-key"));
    let kwargs = kwargs_of(
        r#"{"tenant_id":"tenant","client_secret":"secret","max_retries":5,"timeout":"7.5","azure_ad_token":"param-token"}"#,
    );
    let prepared = azure_prepare(&harness, &kwargs, None);
    let Target::Azure {
        request, client, ..
    } = prepared.target
    else {
        panic!("azure prepares an azure client");
    };
    assert_eq!(request.api_base.as_deref(), Some("https://azure.test"));
    assert_eq!(request.api_version.as_deref(), Some("2024-02-01"));
    assert!(request.azure_ad_token.is_none());
    assert_eq!(
        request.params.azure_ad_token.as_ref().map(secret_text),
        Some(String::from("param-token"))
    );
    assert_eq!(request.params.auth.tenant_id.as_deref(), Some("tenant"));
    assert_eq!(request.params.max_retries, Some(5));
    assert_eq!(request.params.timeout, Some(7.5_f64));
    assert_eq!(client.kind, ClientKind::Sync);
    assert_eq!(client.api_version, "2024-02-01");
    assert_eq!(client.deadline, Duration::from_millis(7500));
}

#[rstest]
#[case::absent("", "")]
#[case::empty_body("{}", "")]
#[case::null_body("null", "env-token")]
#[case::body_token(r#"{"azure_ad_token":"body-token"}"#, "body-token")]
#[test]
fn azure_token_sources_are_absent_null_or_body(
    #[case] extra: &'static str,
    #[case] expected: &'static str,
) {
    let mut harness = Harness::new();
    harness.resources.azure_ad_token = Some(SecretString::from("env-token"));
    let mut kwargs = BTreeMap::new();
    if !extra.is_empty() {
        kwargs.insert(String::from("extra_body"), json(extra));
    }
    let prepared = azure_prepare(&harness, &kwargs, None);
    let Target::Azure { request, .. } = prepared.target else {
        panic!("azure prepares an azure client");
    };
    let wanted = if expected.is_empty() {
        None
    } else {
        Some(String::from(expected))
    };
    assert_eq!(request.azure_ad_token.as_ref().map(secret_text), wanted);
}

fn azure_client(
    kind: ClientKind,
    transport: Arc<Recorder>,
) -> AzureAssistantsClient<'static, Recorder> {
    AzureAssistantsClient {
        kind,
        inputs: None,
        transport,
        base_url: String::from("https://supplied-azure.test"),
        api_version: String::from("2024-01-01"),
        key: Some(SecretString::from("supplied-azure-key")),
        token: None,
        deadline: Duration::from_secs(42),
    }
}

#[rstest]
#[case::openai_sync(false, ClientKind::Sync, "azure:Sync:None")]
#[case::openai_async(false, ClientKind::Asynchronous, "openai:Asynchronous")]
#[case::azure_sync(true, ClientKind::Sync, "azure:Sync:None")]
#[case::azure_async(true, ClientKind::Asynchronous, "azure:Asynchronous:None")]
#[tokio::test]
async fn azure_drops_the_sync_sdk_classes_only(
    #[case] azure: bool,
    #[case] kind: ClientKind,
    #[case] expected: &'static str,
) {
    let harness = Harness::new();
    let capture = Capture::default();
    let mut kwargs = BTreeMap::new();
    let mut call = create_call(&mut kwargs, None);
    call.custom_llm_provider = String::from("azure");
    call.client = Some(if azure {
        SuppliedClient::Azure(azure_client(kind, Arc::default()))
    } else {
        SuppliedClient::Openai(SdkClient {
            kind,
            api_key: Some(SecretString::from("supplied-key")),
            base_url: Some(String::from("https://supplied.test/v1")),
            timeout: Some(Duration::from_secs(42)),
            max_retries: Some(0),
            organization: None,
            transport: Arc::default(),
        })
    });
    let thread = execute(&harness.threads(), FacadeCall::Create(call), &capture)
        .await
        .expect("the call prepares");
    assert!(thread.is_none());
    assert_eq!(capture.calls.lock().unwrap()[0], expected);
}

#[tokio::test]
async fn azure_create_posts_to_threads_with_api_key_auth() {
    let mut harness = Harness::new();
    harness.resources.azure_api_base = Some(String::from("https://azure.test"));
    harness.resources.azure_api_version = Some(String::from("2024-02-01"));
    harness.resources.azure_openai_api_key = Some(SecretString::from("azure-key"));
    harness.recorder.queue(200, THREAD);
    let mut kwargs = BTreeMap::new();
    let mut call = create_call(&mut kwargs, Some(vec![message("hello")]));
    call.custom_llm_provider = String::from("azure");
    let thread = harness
        .threads()
        .create_thread(call)
        .await
        .expect("the create succeeds")
        .expect("the thread returns");
    assert_eq!(thread.id, "thread_a");
    let seen = &harness.recorder.seen()[0];
    assert_eq!(seen.method, "POST");
    assert_eq!(
        seen.url,
        "https://azure.test/openai/threads?api-version=2024-02-01"
    );
    assert_eq!(seen.api_key.as_deref(), Some("azure-key"));
    assert_eq!(seen.beta.as_deref(), Some("assistants=v2"));
    assert_eq!(
        seen.body,
        json(r#"{"messages":[{"role":"user","content":"hello"}],"metadata":{"tag":"x"}}"#)
    );
}

#[tokio::test]
async fn a_facade_extra_body_token_without_a_key_is_refused_before_the_request() {
    let mut harness = Harness::new();
    harness.resources.azure_openai_api_key = None;
    harness.resources.azure_ad_token = Some(SecretString::from("env-token"));
    let mut kwargs = kwargs_of(r#"{"extra_body":null}"#);
    let mut call = create_call(&mut kwargs, None);
    call.custom_llm_provider = String::from("azure");
    let failure = harness
        .threads()
        .acreate_thread(call)
        .await
        .expect_err("the facade token never reaches the azure client");
    assert!(failure.error.to_string().contains("Missing credentials"));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_sdk_failure(&failure, 500, "APIConnectionError");
    assert!(harness.recorder.seen().is_empty());
}

#[tokio::test]
async fn azure_sends_the_top_level_token_as_bearer_auth_without_a_key() {
    let mut harness = Harness::new();
    harness.resources.azure_openai_api_key = None;
    harness.recorder.queue(200, THREAD);
    let mut kwargs = kwargs_of(r#"{"azure_ad_token":"param-token"}"#);
    let mut call = create_call(&mut kwargs, None);
    call.custom_llm_provider = String::from("azure");
    let thread = harness
        .threads()
        .create_thread(call)
        .await
        .expect("the create succeeds");
    assert!(thread.is_some());
    let seen = &harness.recorder.seen()[0];
    assert_eq!(seen.authorization.as_deref(), Some("Bearer param-token"));
    assert_eq!(seen.api_key, None);
}

#[tokio::test]
async fn an_azure_provider_404_maps_without_body_fields_asynchronously() {
    let mut harness = Harness::new();
    harness.resources.azure_api_base = Some(String::from("https://azure.test"));
    harness.resources.azure_openai_api_key = Some(SecretString::from("azure-key"));
    harness.recorder.queue(404, UNTYPED_404);
    let mut kwargs = BTreeMap::new();
    let call = GetThreadCall {
        custom_llm_provider: String::from("azure"),
        thread_id: String::from("thread_a"),
        client: None,
        kwargs: &mut kwargs,
    };
    let failure = harness
        .threads()
        .aget_thread(call)
        .await
        .expect_err("the mock failed");
    let metadata = failure.class.metadata();
    assert!(matches!(failure.class, ErrorClass::NotFound(_)));
    assert_eq!(metadata.status_code, 404);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert_eq!(harness.recorder.seen().len(), 1);
}

#[rstest]
#[case::synchronous(false)]
#[case::asynchronous(true)]
#[test]
fn a_supplied_azure_client_is_kept_only_when_it_is_async(#[case] asynchronous: bool) {
    let harness = Harness::new();
    let kind = if asynchronous {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    };
    let expected = if asynchronous {
        "https://supplied-azure.test"
    } else {
        "https://azure.test"
    };
    let kwargs = BTreeMap::new();
    let prepared = azure_prepare(
        &harness,
        &kwargs,
        Some(SuppliedClient::Azure(azure_client(kind, Arc::default()))),
    );
    let Target::Azure { client, .. } = prepared.target else {
        panic!("azure prepares an azure client");
    };
    assert!(client.base_url.starts_with(expected), "{}", client.base_url);
}

#[rstest]
#[case::create(false)]
#[case::get(true)]
#[test]
fn an_openai_facade_prepares_a_supplied_azure_client_for_both_operations(#[case] retrieve: bool) {
    let harness = Harness::new();
    let kwargs = BTreeMap::new();
    let operation = if retrieve {
        Operation::Retrieve(String::from("thread_a"))
    } else {
        empty_create()
    };
    let prepared = harness
        .threads()
        .prepare(PrepareRequest {
            provider: String::from("openai"),
            operation,
            client: Some(SuppliedClient::Azure(azure_client(
                ClientKind::Sync,
                Arc::default(),
            ))),
            kwargs: &kwargs,
            facade: ThreadOperation::Create,
            flag: None,
        })
        .expect("the call prepares");
    let Target::Azure { client, .. } = prepared.target else {
        panic!("an openai facade keeps the supplied azure client");
    };
    assert_eq!(client.base_url, "https://supplied-azure.test");
}
