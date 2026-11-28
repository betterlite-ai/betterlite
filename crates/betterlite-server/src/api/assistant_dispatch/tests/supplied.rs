use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_core::api::assistant_run_sdk::service::RunOutcome;
use betterlite_core::api::assistant_threads::SuppliedClient as ThreadClient;
use betterlite_http::client_kind::ClientKind;
use betterlite_models::decisions::Json;

use super::super::client::SuppliedClass;
use super::super::operation::AssistantOperation;
use super::super::outcome::AssistantOutcome;
use super::*;

// arun_thread reads a supplied client and its optional kwargs through the run owner
#[tokio::test]
async fn a_supplied_client_reaches_the_run_owner() {
    let settings = configured("openai", &[]);
    let metadata = object(&[("suite", text("b2489"))]);
    let outcome = call(
        &settings,
        AssistantOperation::RunThread,
        &data(&[
            ("assistant_id", text("asst_a")),
            ("metadata", metadata.clone()),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Route {
            thread_id: Some("thread_a"),
            assistant_id: None,
            client: Some(supplied(
                SuppliedClass::AsyncOpenAi,
                "sk-supplied",
                MOCK_BASE,
                None,
            )),
        },
        &[(200, RUN), (200, RUN)],
    )
    .await;
    let AssistantOutcome::Run(RunOutcome::Polled(reply)) = outcome.result.expect("run succeeds")
    else {
        panic!("run answered with another outcome");
    };
    assert_eq!((*reply).expect("the provider replied").id, "run_a");
    // the supplied client's own key wins over the params key, unchanged
    assert_eq!(
        outcome.seen[0].authorization.as_deref(),
        Some("Bearer sk-supplied")
    );
    let Json::Object(fields) = &outcome.seen[0].body else {
        panic!("the provider body is not an object");
    };
    assert_eq!(fields.get("metadata"), Some(&metadata));
}

// B1057's message owner takes no supplied client yet, so the request's own parameters reach the provider
#[tokio::test]
async fn a_supplied_client_does_not_reach_the_message_owner() {
    let settings = configured("openai", &[]);
    let outcome = call(
        &settings,
        AssistantOperation::CreateMessage,
        &data(&[
            ("role", text("user")),
            ("content", text("hello")),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Route {
            thread_id: Some("thread_a"),
            assistant_id: None,
            client: Some(supplied(
                SuppliedClass::AsyncOpenAi,
                "sk-supplied",
                MOCK_BASE,
                None,
            )),
        },
        &[(200, MESSAGE)],
    )
    .await;
    outcome.result.expect("the message call succeeds");
    assert_eq!(
        outcome.seen[0].authorization.as_deref(),
        Some("Bearer sk-test")
    );
}

// an AsyncAzureOpenAI client keeps its own class, endpoint, key and api_version through create_thread
#[tokio::test]
async fn a_supplied_azure_client_reaches_create_thread() {
    let settings = configured("openai", &[]);
    let outcome = call(
        &settings,
        AssistantOperation::CreateThread,
        &data(&[("custom_llm_provider", text("azure"))]),
        Route {
            thread_id: None,
            assistant_id: None,
            client: Some(supplied(
                SuppliedClass::AsyncAzureOpenAi,
                "sk-azure",
                "https://azure.test",
                Some("2024-05-01-preview"),
            )),
        },
        &[(200, THREAD)],
    )
    .await;
    outcome
        .result
        .expect("the supplied azure client creates the thread");
    let seen = &outcome.seen[0];
    assert_eq!(seen.method, "POST");
    assert_eq!(
        seen.url,
        "https://azure.test/openai/threads?api-version=2024-05-01-preview"
    );
    assert_eq!(seen.api_key.as_deref(), Some("sk-azure"));
    assert_eq!(seen.authorization, None);
}

#[tokio::test]
async fn a_supplied_azure_client_reaches_get_thread() {
    let settings = configured("openai", &[]);
    let outcome = call(
        &settings,
        AssistantOperation::GetThread,
        &data(&[("custom_llm_provider", text("azure"))]),
        Route {
            thread_id: Some("thread_a"),
            assistant_id: None,
            client: Some(supplied(
                SuppliedClass::AsyncAzureOpenAi,
                "sk-azure",
                "https://azure.test",
                Some("2024-05-01-preview"),
            )),
        },
        &[(200, THREAD)],
    )
    .await;
    outcome
        .result
        .expect("the supplied azure client reads the thread");
    let seen = &outcome.seen[0];
    assert_eq!(seen.method, "GET");
    assert_eq!(
        seen.url,
        "https://azure.test/openai/threads/thread_a?api-version=2024-05-01-preview"
    );
    assert_eq!(seen.api_key.as_deref(), Some("sk-azure"));
    assert_eq!(seen.authorization, None);
}

// the client it builds carries the supplied AsyncAzureOpenAI class into the SDK client's own inputs
#[test]
fn a_supplied_azure_client_keeps_its_class_in_the_thread_client() {
    let client = supplied(
        SuppliedClass::AsyncAzureOpenAi,
        "sk-azure",
        "https://azure.test",
        Some("2024-05-01-preview"),
    );
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let thread = client
        .thread(Arc::new(Recorder::default()), &environment, "2024-01-01")
        .expect("the supplied azure client builds");
    let ThreadClient::Azure(azure) = thread else {
        panic!("the supplied azure class built another client");
    };
    assert_eq!(azure.kind, ClientKind::Asynchronous);
    let inputs = azure.inputs.expect("the constructor inputs");
    assert!(inputs.http_client.is_async);
}
