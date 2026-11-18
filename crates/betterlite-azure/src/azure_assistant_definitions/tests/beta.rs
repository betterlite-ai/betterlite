#![cfg(test)]

use betterlite_openai::assistants::records::AssistantCreate;

use crate::azure_assistant_definitions::operations::ClientRequest;

use super::support::{
    ASSISTANT, Auth, CREATE_BODY, DELETED, Fixture, KEY, PAGE, Seen, harness, wire,
};

const DEFAULT_VERSION: &str = "2025-02-01-preview";

pub(super) async fn check() {
    default_version().await;
    environment_version().await;
    trailing_slash().await;
    deployments().await;
    base_query_version().await;
}

// Every definition call carries the shared beta namespace header and the B531 endpoint query.
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

// The SDK merges the base URL's query with api-version, and api-version wins when both name it.
async fn base_query_version() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from(
        "https://example.test/openai/deployments/dep?api-version=old",
    ));
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.get_assistants(&request, None, None).await.unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            "https://example.test/openai/deployments/dep?api-version=2024-02-01",
            None,
            Auth::Key(KEY)
        )]
    );
}

async fn exchange(request: &ClientRequest<'_>) -> Vec<Seen> {
    let data: AssistantCreate = serde_json::from_str(CREATE_BODY).unwrap();
    let (list_api, list_transport) = harness(200, PAGE);
    list_api.get_assistants(request, None, None).await.unwrap();
    let (create_api, create_transport) = harness(200, ASSISTANT);
    create_api
        .create_assistants(request, &data, None, None)
        .await
        .unwrap();
    let (delete_api, delete_transport) = harness(200, DELETED);
    delete_api
        .delete_assistant(request, "asst_1", None, None)
        .await
        .unwrap();
    let mut seen = list_transport.requests();
    seen.extend(create_transport.requests());
    seen.extend(delete_transport.requests());
    seen
}

fn expect(seen: &[Seen], base: &str, version: &str) {
    let assistants = format!("{base}/assistants?api-version={version}");
    let gone = format!("{base}/assistants/asst_1?api-version={version}");
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0], wire("GET", &assistants, None, Auth::Key(KEY)));
    assert_eq!(
        seen[1],
        wire("POST", &assistants, Some(CREATE_BODY), Auth::Key(KEY))
    );
    assert_eq!(seen[2], wire("DELETE", &gone, None, Auth::Key(KEY)));
}
