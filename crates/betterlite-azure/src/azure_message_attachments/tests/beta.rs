#![cfg(test)]

use crate::azure_message_attachments::operations::{MessageCall, MessageReply};

use super::support::{Auth, Fixture, KEY, MESSAGE, MESSAGE_DATA, PAGE, data, harness, wire};

const DEFAULT_VERSION: &str = "2025-02-01-preview";

// Append and list both carry the shared beta namespace header and the B686 endpoint query.
pub(super) async fn check() {
    default_version().await;
    environment_version().await;
    deployments().await;
    base_query().await;
}

async fn default_version() {
    let url = default_version_append().await;
    default_version_list(&url).await;
}

async fn default_version_append() -> String {
    let mut fixture = Fixture::new();
    fixture.api_version = None;
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE);
    api.call(
        &request,
        None,
        None,
        MessageCall::Append {
            thread_id: "thread_1",
            data: &data,
        },
    )
    .await
    .unwrap();
    let url = format!(
        "https://example.test/openai/threads/thread_1/messages?api-version={DEFAULT_VERSION}"
    );
    assert_eq!(
        transport.requests(),
        vec![wire("POST", &url, Some(MESSAGE_DATA), Auth::Key(KEY))]
    );
    url
}

async fn default_version_list(url: &str) {
    let mut fixture = Fixture::new();
    fixture.api_version = None;
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let reply = api
        .call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    let MessageReply::Page(page) = reply.reply else {
        panic!("list returns a page");
    };
    assert!(page.data.is_empty());
    assert_eq!(
        transport.requests(),
        vec![wire("GET", url, None, Auth::Key(KEY))]
    );
    assert_eq!(
        transport.requests()[0]
            .headers
            .get("openai-beta")
            .map(String::as_str),
        Some("assistants=v2")
    );
}

async fn environment_version() {
    let mut fixture = Fixture::new();
    fixture.api_version = None;
    fixture.environment.insert(
        String::from("AZURE_API_VERSION"),
        String::from("2024-06-01"),
    );
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            "https://example.test/openai/threads/thread_1/messages?api-version=2024-06-01",
            None,
            Auth::Key(KEY)
        )]
    );
}

async fn deployments() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("https://example.test/openai/deployments/dep/"));
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            "https://example.test/openai/deployments/dep/threads/thread_1/messages?api-version=2024-02-01",
            None,
            Auth::Key(KEY)
        )]
    );
}

// The SDK merges the base URL's query with api-version, and api-version wins when both name it.
async fn base_query() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from(
        "https://example.test/openai/deployments/dep?api-version=old",
    ));
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            "https://example.test/openai/deployments/dep?api-version=2024-02-01",
            None,
            Auth::Key(KEY)
        )]
    );
    let mut swallowed = Fixture::new();
    swallowed.api_base = Some(String::from("https://example.test/openai?foo=1"));
    let second_request = swallowed.request();
    let (second_api, second_transport) = harness(200, PAGE);
    second_api
        .call(&second_request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        second_transport.requests(),
        vec![wire(
            "GET",
            "https://example.test/openai?foo=1%2Fopenai%2Fthreads%2Fthread_1%2Fmessages&api-version=2024-02-01",
            None,
            Auth::Key(KEY)
        )]
    );
}
