#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_openai::threads::records::ThreadCreate;
use secrecy::SecretString;

use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_thread_messages::error::ThreadMessagesError;
use crate::azure_thread_messages::operations::ThreadOperation;
use crate::credentials::source::CallableTokenProvider;
use crate::endpoints::projection::CallableProvider;

use super::support::{
    Auth, ERROR_BODY, EmptyToken, FailingToken, FixedToken, Fixture, KEY, Seen, THREAD, THREAD_URL,
    THREADS_URL, failing_harness, harness, status_of, supplied_client, wire,
};

const KINDS: [(Option<bool>, ClientKind); 3] = [
    (None, ClientKind::Sync),
    (Some(false), ClientKind::Sync),
    (Some(true), ClientKind::Asynchronous),
];

pub(super) async fn check() {
    kinds().await;
    supplied_create().await;
    supplied_retrieve().await;
    ignored_arguments().await;
    tokens().await;
    static_token_wins().await;
    environment_token().await;
    provider_with_key().await;
    provider_alone().await;
    provider_failures().await;
    status_failure().await;
    transport_failure().await;
    construction_failure().await;
}

// The flag picks the client for both thread calls; only the literal true makes it async.
async fn kinds() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let create: ThreadCreate = serde_json::from_str("{}").unwrap();
    for (flag, kind) in KINDS {
        let (api, transport) = harness(200, THREAD);
        let created = api
            .call(&request, None, flag, ThreadOperation::Create(&create))
            .await
            .unwrap();
        assert_eq!(created.kind, kind, "{flag:?}");
        assert_eq!(
            transport.requests(),
            vec![wire("POST", THREADS_URL, Some("{}"), Auth::Key(KEY))],
            "{flag:?}"
        );
        let (read_api, read_transport) = harness(200, THREAD);
        let retrieved = read_api
            .call(&request, None, flag, ThreadOperation::Retrieve("thread_1"))
            .await
            .unwrap();
        assert_eq!(retrieved.kind, kind, "{flag:?}");
        assert_eq!(
            read_transport.requests(),
            vec![wire("GET", THREAD_URL, None, Auth::Key(KEY))],
            "{flag:?}"
        );
    }
}

// A supplied client is returned as-is: its own base, version, key and deadline serve the call, and the facade flag never overrides its kind.
async fn supplied_create() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.api_base = None;
    fixture.api_version = None;
    let request = fixture.request();
    let create: ThreadCreate = serde_json::from_str("{}").unwrap();
    for flag in [None, Some(true)] {
        let (api, transport) = harness(200, THREAD);
        let created = api
            .call(
                &request,
                Some(supplied_client(ClientKind::Sync, &transport)),
                flag,
                ThreadOperation::Create(&create),
            )
            .await
            .unwrap();
        assert_eq!(created.kind, ClientKind::Sync, "{flag:?}");
        assert_eq!(
            transport.requests(),
            vec![supplied_wire("POST", "threads", Some("{}"))]
        );
    }
}

async fn supplied_retrieve() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.api_base = None;
    fixture.api_version = None;
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    let retrieved = api
        .call(
            &request,
            Some(supplied_client(ClientKind::Asynchronous, &transport)),
            Some(true),
            ThreadOperation::Retrieve("thread_1"),
        )
        .await
        .unwrap();
    assert_eq!(retrieved.kind, ClientKind::Asynchronous);
    assert_eq!(
        transport.requests(),
        vec![supplied_wire("GET", "threads/thread_1", None)]
    );
}

fn supplied_wire(method: &str, tail: &str, body: Option<&str>) -> Seen {
    let url = format!("https://supplied.test/openai/{tail}?api-version=2024-01-01");
    let mut seen = wire(method, &url, body, Auth::Key("supplied-key"));
    seen.deadline = Duration::from_secs(42);
    seen
}

// The facade's own azure_ad_token, timeout and max_retries are accepted and never forwarded.
async fn ignored_arguments() {
    let mut fixture = Fixture::new();
    fixture.azure_ad_token = Some(SecretString::from("facade-token"));
    fixture.timeout = 1.0_f64;
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    api.call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire("GET", THREAD_URL, None, Auth::Key(KEY))]
    );
    assert_eq!(transport.requests()[0].deadline, Duration::from_secs(600));
}

// A litellm_params token is a static AAD token: it replaces the key with the bearer.
async fn tokens() {
    let mut fixture = Fixture::new();
    fixture.params.azure_ad_token = Some(SecretString::from("params-token"));
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    api.call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire("GET", THREAD_URL, None, Auth::Bearer("params-token"))]
    );
}

// A static token wins over a provider, which is only consulted when the call carries no token.
async fn static_token_wins() {
    let mut fixture = Fixture::new();
    fixture.params.azure_ad_token = Some(SecretString::from("params-token"));
    fixture.provider = Some(provider(Arc::new(FailingToken)));
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    api.call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire("GET", THREAD_URL, None, Auth::Bearer("params-token"))]
    );
}

// The environment supplies the token when the call has none, and a litellm_params timeout sets the deadline.
async fn environment_token() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.params.timeout = Some(30.0_f64);
    fixture.environment.insert(
        String::from("AZURE_OPENAI_AD_TOKEN"),
        String::from("env-token"),
    );
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    api.call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
        .await
        .unwrap();
    let mut expected = wire("GET", THREAD_URL, None, Auth::Bearer("env-token"));
    expected.deadline = Duration::from_secs(30);
    assert_eq!(transport.requests(), vec![expected]);
}

// A provider token rides as the bearer next to the key, since the SDK adds the key separately.
async fn provider_with_key() {
    let mut fixture = Fixture::new();
    fixture.provider = Some(provider(Arc::new(FixedToken("provider-token"))));
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    api.call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            THREAD_URL,
            None,
            Auth::Both {
                key: KEY,
                bearer: "provider-token"
            }
        )]
    );
}

async fn provider_alone() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.provider = Some(provider(Arc::new(FixedToken("provider-token"))));
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    api.call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            THREAD_URL,
            None,
            Auth::Bearer("provider-token")
        )]
    );
}

// A token source that answers no text or fails stops the call before a request is built, with the SDK's error.
async fn provider_failures() {
    let unanswered = provider_error(provider(Arc::new(EmptyToken))).await;
    assert!(
        matches!(
            unanswered,
            ThreadMessagesError::Client(DefinitionsError::ProviderReply)
        ),
        "{unanswered:?}"
    );
    let rejected = provider_error(provider(Arc::new(FailingToken))).await;
    assert!(
        matches!(
            &rejected,
            ThreadMessagesError::Client(DefinitionsError::Provider { class, .. })
                if class == "ClientAuthenticationError"
        ),
        "{rejected:?}"
    );
}

async fn provider_error(provider: CallableProvider) -> ThreadMessagesError {
    let mut fixture = Fixture::new();
    fixture.provider = Some(provider);
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    let failed = api
        .call(&request, None, None, ThreadOperation::Retrieve("thread_1"))
        .await
        .unwrap_err();
    assert!(transport.requests().is_empty());
    failed
}

fn provider(token: Arc<dyn CallableTokenProvider>) -> CallableProvider {
    CallableProvider {
        provider: token,
        identity: 1,
    }
}

// Call failures surface unchanged with their reply body; neither operation catches or retries them.
async fn status_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let create: ThreadCreate = serde_json::from_str("{}").unwrap();
    for status in [400, 408, 409, 429, 500, 503] {
        let (api, transport) = harness(status, ERROR_BODY);
        let failed = api
            .call(&request, None, Some(true), ThreadOperation::Create(&create))
            .await
            .unwrap_err();
        let (seen, seen_body) = status_of(failed);
        assert_eq!(seen, status);
        assert_eq!(seen_body, ERROR_BODY.as_bytes(), "{status}");
        assert_eq!(transport.requests().len(), 1, "{status}");
    }
}

async fn transport_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = failing_harness();
    let failed = api
        .call(
            &request,
            None,
            Some(true),
            ThreadOperation::Retrieve("thread_1"),
        )
        .await;
    assert!(
        matches!(
            failed,
            Err(ThreadMessagesError::Threads(ThreadsError::Transport { .. }))
        ),
        "{failed:?}"
    );
    assert_eq!(transport.requests().len(), 1);
}

async fn construction_failure() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    let request = fixture.request();
    let (api, transport) = harness(200, THREAD);
    let failed = api
        .call(
            &request,
            None,
            Some(true),
            ThreadOperation::Retrieve("thread_1"),
        )
        .await;
    assert!(
        matches!(
            failed,
            Err(ThreadMessagesError::Client(
                DefinitionsError::MissingCredentials
            ))
        ),
        "{failed:?}"
    );
    assert!(transport.requests().is_empty());
}
