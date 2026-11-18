use std::sync::{Arc, Mutex, PoisonError};

use aws_smithy_runtime_api::client::http::{
    HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector, http_client_fn,
};
use aws_smithy_runtime_api::client::orchestrator::HttpRequest;
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;

use super::support::{
    ACCESS, FakeCredentials, Fixture, Options, REGION, RESOLVED_ACCESS, RESOLVED_SECRET,
    RESOLVED_TOKEN, SECRET, TOKEN, auth, credentials, full_auth,
};
use crate::aws_settings::AuthError;
use crate::credentials::error::CredentialError;
use crate::realtime_transport::error::NO_CREDENTIALS_MESSAGE;
use crate::realtime_transport::runtime::{
    RealtimeClient, RealtimeRuntime, RuntimeConfig, SdkRuntime,
};

pub(super) async fn python_sdk_boundary() {
    the_prepared_config_reaches_the_seam().await;
    the_sdk_client_keeps_the_injected_connector().await;
    the_sdk_request_is_signed_once().await;
}

async fn the_prepared_config_reaches_the_seam() {
    let mut fixture = Fixture::new();
    let model = "us.amazon.nova-sonic-v1:0";
    let result = fixture
        .run(Options {
            model,
            api_base: Some("https://bedrock-runtime.us-east-1.amazonaws.com"),
            auth: auth(ACCESS, SECRET, Some(TOKEN)),
            ..Options::default()
        })
        .await;
    assert!(result.outcome.is_ok(), "{:?}", result.outcome);
    let config = fixture.last_config();
    assert_eq!(
        config.endpoint_uri,
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
    assert_eq!(config.region, REGION);
    assert_eq!(config.access_key_id, RESOLVED_ACCESS);
    assert_eq!(config.secret_access_key, RESOLVED_SECRET);
    assert_eq!(config.session_token.as_deref(), Some(RESOLVED_TOKEN));
    assert_eq!(fixture.runtime.invocations(), vec![model.to_owned()]);
}

#[derive(Debug)]
struct RecordingConnector {
    calls: Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>,
}

impl HttpConnector for RecordingConnector {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let uri = request.uri().to_owned();
        let headers = request
            .headers()
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((uri, headers));
        HttpConnectorFuture::new(async move {
            Err(ConnectorError::io(Box::new(std::io::Error::other(
                "the fixture connector refuses every exchange",
            ))))
        })
    }
}

struct Boundary {
    calls: Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>,
    runtime: SdkRuntime,
}

fn boundary() -> Boundary {
    let calls: Arc<Mutex<Vec<(String, Vec<(String, String)>)>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&calls);
    let http = http_client_fn(
        move |_settings: &HttpConnectorSettings, _components: &RuntimeComponents| {
            SharedHttpConnector::new(RecordingConnector {
                calls: Arc::clone(&recorded),
            })
        },
    );
    Boundary {
        runtime: SdkRuntime::new(http),
        calls,
    }
}

fn signed_config() -> RuntimeConfig {
    RuntimeConfig {
        endpoint_uri: "https://bedrock-runtime.eu-west-1.amazonaws.com".to_owned(),
        region: "eu-west-1".to_owned(),
        credentials: credentials(
            "AKIDSIGNEDFIXTURE",
            "signed/secret/+EXAMPLEKEY000",
            Some("signed-session-token"),
        ),
    }
}

fn header_value<'headers>(
    headers: &'headers [(String, String)],
    name: &str,
) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

async fn the_sdk_client_keeps_the_injected_connector() {
    let boundary = boundary();
    let sdk_config = boundary.runtime.config(&signed_config());
    assert_eq!(
        sdk_config.region().map(|region| region.as_ref().to_owned()),
        Some("eu-west-1".to_owned())
    );
    assert_eq!(
        sdk_config.retry_config().map(|retry| retry.max_attempts()),
        Some(1),
        "the SDK retries are disabled"
    );
    assert!(
        sdk_config.http_client().is_some(),
        "the injected client is kept"
    );
}

async fn the_sdk_request_is_signed_once() {
    let boundary = boundary();
    let client = boundary
        .runtime
        .connect(signed_config())
        .await
        .expect("the SDK client builds");
    let outcome = client.invoke("eu.amazon.nova-sonic-v1:0").await;
    assert!(
        outcome.is_err(),
        "the fixture connector refuses the exchange"
    );
    let calls = boundary
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        calls.len(),
        1,
        "a disabled retry config makes exactly one exchange"
    );
    let (uri, headers) = &calls[0];
    assert!(
        uri.starts_with("https://bedrock-runtime.eu-west-1.amazonaws.com/model/"),
        "{uri}"
    );
    assert!(uri.contains("eu.amazon.nova-sonic-v1"), "{uri}");
    assert!(uri.ends_with("/invoke-with-bidirectional-stream"), "{uri}");
    let authorization = header_value(headers, "authorization").expect("the request is signed");
    assert!(
        authorization.contains("Credential=AKIDSIGNEDFIXTURE/"),
        "{authorization}"
    );
    assert!(authorization.contains("eu-west-1"), "{authorization}");
    assert_eq!(
        header_value(headers, "x-amz-security-token"),
        Some("signed-session-token"),
        "the session token comes from the resolved credentials"
    );
}

pub(super) async fn credential_identity_isolation() {
    let mut fixture = Fixture::new();
    fixture.credentials = FakeCredentials::echoing();
    let first = fixture
        .run(Options {
            auth: auth(
                "AKIDFIRSTIDENTITY",
                "first-identity-secret",
                Some("first-identity-token"),
            ),
            ..Options::default()
        })
        .await;
    assert!(first.outcome.is_ok(), "{:?}", first.outcome);
    let second = fixture
        .run(Options {
            auth: auth(
                "AKIDSECONDIDENTITY",
                "second-identity-secret",
                Some("second-identity-token"),
            ),
            region: Some("eu-west-2"),
            ..Options::default()
        })
        .await;
    assert!(second.outcome.is_ok(), "{:?}", second.outcome);
    let calls = fixture.credentials.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].access_key_id.as_deref(), Some("AKIDFIRSTIDENTITY"));
    assert_eq!(
        calls[1].access_key_id.as_deref(),
        Some("AKIDSECONDIDENTITY")
    );
    assert_isolated_identities(&fixture);
}

fn assert_isolated_identities(fixture: &Fixture) {
    let configs = fixture.runtime.configs();
    assert_eq!(configs.len(), 2);
    assert_eq!(configs[0].access_key_id, "AKIDFIRSTIDENTITY");
    assert_eq!(configs[0].secret_access_key, "first-identity-secret");
    assert_eq!(
        configs[0].session_token.as_deref(),
        Some("first-identity-token")
    );
    assert_eq!(configs[0].region, REGION);
    assert_eq!(configs[1].access_key_id, "AKIDSECONDIDENTITY");
    assert_eq!(configs[1].secret_access_key, "second-identity-secret");
    assert_eq!(
        configs[1].session_token.as_deref(),
        Some("second-identity-token")
    );
    assert_eq!(configs[1].region, "eu-west-2");
}

pub(super) async fn secret_safe_failure() {
    let mut fixture = Fixture::new();
    fixture.credentials = FakeCredentials::failing(CredentialError::Auth(AuthError::SessionTags));
    let result = fixture
        .run(Options {
            auth: full_auth(),
            ..Options::default()
        })
        .await;
    let calls = fixture.credentials.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].access_key_id.as_deref(),
        Some(ACCESS),
        "the fixture secrets really reached the resolver"
    );
    assert_eq!(calls[0].secret_access_key.as_deref(), Some(SECRET));
    assert_eq!(calls[0].session_token.as_deref(), Some(TOKEN));
    let error = result.outcome.unwrap_err();
    let rendered = format!("{error} {error:?}");
    assert!(rendered.contains("aws_session_tags"), "{rendered}");
    for secret in [ACCESS, SECRET, TOKEN, "fixture-web-identity"] {
        assert!(
            !rendered.contains(secret),
            "the failure leaks {secret}: {rendered}"
        );
    }
    absent_credentials_never_leak().await;
}

async fn absent_credentials_never_leak() {
    let mut absent = Fixture::new();
    absent.credentials = FakeCredentials::absent();
    let result = absent
        .run(Options {
            auth: full_auth(),
            ..Options::default()
        })
        .await;
    let error = result.outcome.unwrap_err();
    assert_eq!(error.to_string(), NO_CREDENTIALS_MESSAGE);
    let rendered = format!("{error:?}");
    for secret in [ACCESS, SECRET, TOKEN, "fixture-web-identity"] {
        assert!(
            !rendered.contains(secret),
            "the 401 leaks {secret}: {rendered}"
        );
    }
}
