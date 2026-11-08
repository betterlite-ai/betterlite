use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::timeout;

use crate::gcs_mock::client::{
    GcsHttpClient as _, GcsMockClient, GcsMockError, create_mock_gcs_client,
};
use crate::gcs_mock::installation::Installation;
use crate::gcs_mock::payload::{MOCK_LOGGING_PAYLOAD, MOCK_STORAGE_OBJECT};
use crate::gcs_mock::request::{GcsMethod, GcsRequest};
use crate::gcs_mock::settings::{ENABLE_ENV, GcsMockSettings, LATENCY_ENV};

use super::support::{FailingClient, RecordingClient, environment, request};

const STORAGE_URL: &str =
    "https://storage.googleapis.com/storage/v1/b/mock-bucket/o/mock-object?alt=media";
const UPLOAD_URL: &str = "https://storage.googleapis.com/upload/storage/v1/b/mock-bucket/o";

fn recorder(status: u16, body: &str) -> (RecordingClient, Arc<Mutex<Vec<GcsRequest>>>) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    (RecordingClient::new(Arc::clone(&sent), status, body), sent)
}

fn recorded(sent: &Arc<Mutex<Vec<GcsRequest>>>) -> Vec<GcsRequest> {
    sent.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

#[tokio::test(start_paused = true)]
async fn get_to_storage_returns_the_canned_payload() {
    let (original, sent) = recorder(418, "original-response");
    let client =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), Some(original)).unwrap();
    let response = client
        .send(request(STORAGE_URL, GcsMethod::Get))
        .await
        .expect("mock response");
    assert_eq!(response.status, 200);
    assert_eq!(response.url, STORAGE_URL);
    assert_eq!(
        response.body,
        serde_json::to_vec(&MOCK_LOGGING_PAYLOAD).unwrap()
    );
    assert_eq!(response.elapsed, Duration::from_millis(150));
    assert!(recorded(&sent).is_empty());
}

#[tokio::test(start_paused = true)]
async fn get_to_other_hosts_delegates_unchanged() {
    let (original, sent) = recorder(418, "original-response");
    let client =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), Some(original)).unwrap();
    let request = request(
        "https://example.com/storage/v1/b/mock-bucket/o/mock-object",
        GcsMethod::Get,
    );
    let response = client
        .send(request.clone())
        .await
        .expect("delegated response");
    assert_eq!(response.status, 418);
    assert_eq!(response.body, b"original-response".to_vec());
    assert_eq!(recorded(&sent), vec![request]);
}

#[tokio::test(start_paused = true)]
async fn post_to_storage_returns_the_factory_response() {
    let (original, sent) = recorder(418, "original-response");
    let client =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), Some(original)).unwrap();
    let response = client
        .send(request(UPLOAD_URL, GcsMethod::Post))
        .await
        .expect("mock response");
    assert_eq!(response.status, 200);
    assert_eq!(response.url, UPLOAD_URL);
    assert_eq!(
        response.body,
        serde_json::to_vec(&MOCK_STORAGE_OBJECT).unwrap()
    );
    assert!(recorded(&sent).is_empty());
}

#[tokio::test(start_paused = true)]
async fn post_to_other_hosts_delegates_unchanged() {
    let (original, sent) = recorder(418, "original-response");
    let client =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), Some(original)).unwrap();
    let request = request("https://example.com/upload", GcsMethod::Post);
    let response = client
        .send(request.clone())
        .await
        .expect("delegated response");
    assert_eq!(response.status, 418);
    assert_eq!(recorded(&sent), vec![request]);
}

#[tokio::test(start_paused = true)]
async fn delete_to_storage_returns_204_with_an_empty_body() {
    let (original, sent) = recorder(418, "original-response");
    let client =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), Some(original)).unwrap();
    let response = client
        .send(request(STORAGE_URL, GcsMethod::Delete))
        .await
        .expect("mock response");
    assert_eq!(response.status, 204);
    assert!(response.body.is_empty());
    assert!(recorded(&sent).is_empty());
}

#[tokio::test(start_paused = true)]
async fn delete_to_other_hosts_delegates_unchanged() {
    let (original, sent) = recorder(418, "original-response");
    let client =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), Some(original)).unwrap();
    let request = request("https://example.com/object", GcsMethod::Delete);
    let response = client
        .send(request.clone())
        .await
        .expect("delegated response");
    assert_eq!(response.status, 418);
    assert_eq!(recorded(&sent), vec![request]);
}

#[tokio::test(start_paused = true)]
async fn post_matching_ignores_url_case() {
    let client: GcsMockClient<RecordingClient> =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), None).unwrap();
    let response = client
        .send(request(
            "https://STORAGE.GOOGLEAPIS.COM/upload",
            GcsMethod::Post,
        ))
        .await
        .expect("mock response");
    assert_eq!(response.status, 200);
}

#[tokio::test(start_paused = true)]
async fn get_matching_is_case_sensitive() {
    let (original, sent) = recorder(418, "original-response");
    let client =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), Some(original)).unwrap();
    let response = client
        .send(request(
            "https://STORAGE.GOOGLEAPIS.COM/object",
            GcsMethod::Get,
        ))
        .await
        .expect("delegated response");
    assert_eq!(response.status, 418);
    assert_eq!(recorded(&sent).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn default_latency_delays_every_response() {
    let client: GcsMockClient<RecordingClient> =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), None).unwrap();
    let early = timeout(
        Duration::from_millis(149),
        client.send(request(STORAGE_URL, GcsMethod::Get)),
    )
    .await;
    early.unwrap_err();
    let response = timeout(
        Duration::from_millis(151),
        client.send(request(STORAGE_URL, GcsMethod::Get)),
    )
    .await
    .expect("completes after the default latency")
    .expect("mock response");
    assert_eq!(response.elapsed, Duration::from_millis(150));
}

#[tokio::test(start_paused = true)]
async fn latency_reads_milliseconds_from_the_environment() {
    let client: GcsMockClient<RecordingClient> = create_mock_gcs_client(
        &environment(&[(ENABLE_ENV, "true"), (LATENCY_ENV, "25")]),
        None,
    )
    .unwrap();
    let early = timeout(
        Duration::from_millis(24),
        client.send(request(STORAGE_URL, GcsMethod::Get)),
    )
    .await;
    early.unwrap_err();
    let response = timeout(
        Duration::from_millis(26),
        client.send(request(STORAGE_URL, GcsMethod::Get)),
    )
    .await
    .expect("completes after the configured latency")
    .expect("mock response");
    assert_eq!(response.elapsed, Duration::from_millis(25));
}

#[tokio::test(start_paused = true)]
async fn latency_applies_to_post_and_delete() {
    let client: GcsMockClient<RecordingClient> =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), None).unwrap();
    let post = timeout(
        Duration::from_millis(149),
        client.send(request(UPLOAD_URL, GcsMethod::Post)),
    )
    .await;
    post.unwrap_err();
    let delete = timeout(
        Duration::from_millis(149),
        client.send(request(STORAGE_URL, GcsMethod::Delete)),
    )
    .await;
    delete.unwrap_err();
}

#[tokio::test(start_paused = true)]
async fn zero_latency_returns_immediately() {
    let client: GcsMockClient<RecordingClient> = create_mock_gcs_client(
        &environment(&[(ENABLE_ENV, "true"), (LATENCY_ENV, "0")]),
        None,
    )
    .unwrap();
    let response = timeout(
        Duration::from_millis(1),
        client.send(request(STORAGE_URL, GcsMethod::Get)),
    )
    .await
    .expect("a zero latency needs no wait")
    .expect("mock response");
    assert_eq!(response.elapsed, Duration::ZERO);
}

#[tokio::test]
async fn missing_original_errors_for_non_gcs_urls() {
    let client: GcsMockClient<RecordingClient> =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), None).unwrap();
    let error = client
        .send(request("https://example.com/object", GcsMethod::Get))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        GcsMockError::MissingOriginal {
            method: GcsMethod::Get
        }
    ));
    assert_eq!(
        error.to_string(),
        "no original HTTP handler is installed for GET"
    );
}

#[tokio::test(start_paused = true)]
async fn disabled_mock_delegates_storage_urls() {
    let (original, sent) = recorder(418, "original-response");
    let client =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "false")]), Some(original)).unwrap();
    let response = client
        .send(request(STORAGE_URL, GcsMethod::Get))
        .await
        .expect("delegated response");
    assert_eq!(response.status, 418);
    assert_eq!(recorded(&sent).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn install_is_idempotent() {
    let (original, sent) = recorder(418, "original-response");
    let mut client = GcsMockClient {
        settings: GcsMockSettings {
            enabled: false,
            latency: Duration::from_millis(150),
        },
        real: Some(original),
        mocks_initialized: false,
    };
    let delegated = client
        .send(request(STORAGE_URL, GcsMethod::Get))
        .await
        .expect("delegated response");
    assert_eq!(delegated.status, 418);
    assert_eq!(client.install(), Installation::Installed);
    let mocked = client
        .send(request(STORAGE_URL, GcsMethod::Get))
        .await
        .expect("mock response");
    assert_eq!(mocked.status, 200);
    assert_eq!(client.install(), Installation::AlreadyInstalled);
    let still_mocked = client
        .send(request(STORAGE_URL, GcsMethod::Get))
        .await
        .expect("mock response");
    assert_eq!(still_mocked.status, 200);
    assert_eq!(recorded(&sent).len(), 1);
}

#[tokio::test]
async fn transport_failures_keep_their_source() {
    let client =
        create_mock_gcs_client(&environment(&[(ENABLE_ENV, "true")]), Some(FailingClient)).unwrap();
    let error = client
        .send(request("https://example.com/object", GcsMethod::Get))
        .await
        .unwrap_err();
    assert!(matches!(error, GcsMockError::Transport { .. }));
    assert!(Error::source(&error).is_some());
}
