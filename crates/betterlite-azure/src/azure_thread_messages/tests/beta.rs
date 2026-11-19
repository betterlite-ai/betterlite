#![cfg(test)]

use betterlite_openai::threads::records::ThreadCreate;

use crate::azure_assistant_definitions::operations::ClientRequest;
use crate::azure_thread_messages::operations::ThreadOperation;

use super::support::{Auth, Fixture, KEY, Seen, THREAD, harness, wire};

const DEFAULT_VERSION: &str = "2025-02-01-preview";

pub(super) async fn check() {
    default_version().await;
    environment_version().await;
    trailing_slash().await;
    deployments().await;
    double_slash_base().await;
    fragment_base().await;
    base_query_version().await;
    base_query_params().await;
}

// Every thread call carries the shared beta namespace header and the B531 endpoint query.
async fn default_version() {
    let mut fixture = Fixture::new();
    fixture.api_version = None;
    let request = fixture.request();
    let seen = exchange(&request).await;
    expect(&seen, "https://example.test/openai", DEFAULT_VERSION);
}

async fn environment_version() {
    let mut fixture = Fixture::new();
    fixture.api_version = None;
    fixture.environment.insert(
        String::from("AZURE_API_VERSION"),
        String::from("2024-06-01"),
    );
    let request = fixture.request();
    let seen = exchange(&request).await;
    expect(&seen, "https://example.test/openai", "2024-06-01");
}

async fn trailing_slash() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("https://example.test/"));
    let request = fixture.request();
    let seen = exchange(&request).await;
    expect(&seen, "https://example.test/openai", "2024-02-01");
}

async fn deployments() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("https://example.test/openai/deployments/dep"));
    let request = fixture.request();
    let seen = exchange(&request).await;
    expect(
        &seen,
        "https://example.test/openai/deployments/dep",
        "2024-02-01",
    );
}

// A base URL ending in two slashes keeps both: the SDK adds a slash only when the raw path lacks one.
async fn double_slash_base() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from(
        "https://example.test/openai/deployments/dep//",
    ));
    let request = fixture.request();
    let seen = exchange(&request).await;
    let write = wire(
        "POST",
        "https://example.test/openai/deployments/dep//threads?api-version=2024-02-01",
        Some("{}"),
        Auth::Key(KEY),
    );
    let read = wire(
        "GET",
        "https://example.test/openai/deployments/dep//threads/thread_1?api-version=2024-02-01",
        None,
        Auth::Key(KEY),
    );
    assert_eq!(seen, vec![write, read]);
}

// The tail goes ahead of a base URL's fragment, which stays after the merged query.
async fn fragment_base() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from(
        "https://example.test/openai/deployments/dep#x",
    ));
    let request = fixture.request();
    let seen = exchange(&request).await;
    let write = wire(
        "POST",
        "https://example.test/openai/deployments/dep/threads?api-version=2024-02-01#x",
        Some("{}"),
        Auth::Key(KEY),
    );
    let read = wire(
        "GET",
        "https://example.test/openai/deployments/dep/threads/thread_1?api-version=2024-02-01#x",
        None,
        Auth::Key(KEY),
    );
    assert_eq!(seen, vec![write, read]);
}

// The SDK's raw-path join keeps the base URL's query, and api-version wins when both name it.
async fn base_query_version() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from(
        "https://example.test/openai/deployments/dep?api-version=old",
    ));
    let request = fixture.request();
    let seen = exchange(&request).await;
    assert_eq!(seen.len(), 2);
    for entry in &seen {
        assert_eq!(
            entry.url,
            "https://example.test/openai/deployments/dep?api-version=2024-02-01"
        );
    }
}

// The tail joins the raw URL, so the last query value of a base URL swallows it exactly as httpx does.
async fn base_query_params() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from(
        "https://example.test/openai/deployments/dep?foo=bar&api-version=old&empty=",
    ));
    let request = fixture.request();
    let seen = exchange(&request).await;
    let write = wire(
        "POST",
        "https://example.test/openai/deployments/dep?foo=bar&api-version=2024-02-01&empty=%2Fthreads",
        Some("{}"),
        Auth::Key(KEY),
    );
    let read = wire(
        "GET",
        "https://example.test/openai/deployments/dep?foo=bar&api-version=2024-02-01&empty=%2Fthreads%2Fthread_1",
        None,
        Auth::Key(KEY),
    );
    assert_eq!(seen, vec![write, read]);
}

async fn exchange(request: &ClientRequest<'_>) -> Vec<Seen> {
    let create: ThreadCreate = serde_json::from_str("{}").unwrap();
    let (create_api, create_transport) = harness(200, THREAD);
    create_api
        .call(request, None, None, ThreadOperation::Create(&create))
        .await
        .unwrap();
    let (retrieve_api, retrieve_transport) = harness(200, THREAD);
    retrieve_api
        .call(request, None, None, ThreadOperation::Retrieve("thread_1"))
        .await
        .unwrap();
    let mut seen = create_transport.requests();
    seen.extend(retrieve_transport.requests());
    seen
}

fn expect(seen: &[Seen], base: &str, version: &str) {
    let threads = format!("{base}/threads?api-version={version}");
    let thread = format!("{base}/threads/thread_1?api-version={version}");
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0], wire("POST", &threads, Some("{}"), Auth::Key(KEY)));
    assert_eq!(seen[1], wire("GET", &thread, None, Auth::Key(KEY)));
}
