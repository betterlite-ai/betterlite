#![cfg(test)]

use super::{
    OutboundRequest, PreparedCall, ProxiedClient, RequestBody, SagemakerEnvironment,
    SagemakerRuntime, SagemakerTransport, header_map,
};
use crate::credentials::CredentialSource;
use crate::sagemaker_runtime::error::RuntimeError;
use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use betterlite_config::{proxy::ProxyPolicy, types::ProxyTrust};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use futures_util::stream;
use http::{HeaderMap, HeaderValue, Method};
use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;

use crate::aws_endpoints::sts::StsClientKwargs;
use crate::aws_settings::credential_cache::CredentialCache;
use crate::aws_settings::sources::NativeSources;
use crate::credentials::AwsCredentialResolver;
use aws_smithy_async::rt::sleep::{SharedAsyncSleep, TokioSleep};
use aws_smithy_runtime_api::client::http::{
    HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn,
};
use aws_smithy_runtime_api::client::orchestrator::HttpRequest;
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_types::os_shim_internal::Fs;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::tls::Verification;
use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_redact::secret::Redactor;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::{ExposeSecret as _, SecretString};
use std::io;
use std::sync::Mutex;

#[derive(Debug)]
struct NoNetwork;

impl HttpConnector for NoNetwork {
    fn call(&self, _request: HttpRequest) -> HttpConnectorFuture {
        HttpConnectorFuture::ready(Err(ConnectorError::other(
            Box::new(io::Error::other("unexpected network access")),
            None,
        )))
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("unexpected OIDC request"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

struct Counter;

fn test_identifier() -> Uuid {
    Uuid::nil()
}

impl TextTokenCounter for Counter {
    fn count_response_tokens(&self, text: &str) -> u64 {
        u64::try_from(text.split_whitespace().count()).unwrap_or_default()
    }
}

#[test]
fn environment_inputs_keep_each_region_source() {
    let environment = SagemakerEnvironment {
        region_name: Some(String::from("eu-west-1")),
        region: Some(String::from("us-east-1")),
        session_region: Some(String::from("ap-south-1")),
    };
    let inputs = environment.inputs();
    assert_eq!(inputs.environment_region_name, Some("eu-west-1"));
    assert_eq!(inputs.environment_region, Some("us-east-1"));
    assert_eq!(inputs.session_region, Some("ap-south-1"));
    assert_eq!(
        SagemakerEnvironment::default().inputs().environment_region,
        None
    );
}

#[test]
fn header_map_validates_names_and_values_and_marks_secrets() {
    let headers = vec![
        (String::from("Authorization"), String::from("signature")),
        (String::from("X-Amz-Security-Token"), String::from("token")),
        (
            String::from("Content-Type"),
            String::from("application/json"),
        ),
    ];
    let map = header_map(&headers).expect("valid headers");
    assert!(
        map.get("authorization")
            .is_some_and(HeaderValue::is_sensitive)
    );
    assert!(
        map.get("x-amz-security-token")
            .is_some_and(HeaderValue::is_sensitive)
    );
    assert!(
        map.get("content-type")
            .is_some_and(|value| !value.is_sensitive())
    );
    assert!(matches!(
        header_map(&[(String::from("bad name"), String::from("value"))]),
        Err(RuntimeError::Header { name }) if name == "bad name"
    ));
    assert!(matches!(
        header_map(&[(String::from("x-test"), String::from("bad\nvalue"))]),
        Err(RuntimeError::Header { name }) if name == "x-test"
    ));
}

#[test]
fn outbound_validates_url_and_builds_post_request() {
    let counter = Counter;
    let runtime = SagemakerRuntime {
        transport: (),
        credentials: (),
        environment: SagemakerEnvironment::default(),
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        counter: &counter,
        timeout: Duration::from_secs(3),
        identifier: test_identifier,
    };
    let prepared = PreparedCall {
        url: String::from("https://runtime.example/invocations"),
        headers: vec![(
            String::from("Content-Type"),
            String::from("application/json"),
        )],
        body: br#"{"inputs":"hi"}"#.to_vec(),
        body_json: IndexMap::from([(String::from("inputs"), Json::Text(String::from("hi")))]),
        region: String::from("us-west-2"),
    };
    let request = runtime.outbound(&prepared).expect("valid outbound request");
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.as_str(), "https://runtime.example/invocations");
    assert_eq!(request.deadline, Duration::from_secs(3));
    assert!(matches!(
        runtime.outbound(&PreparedCall {
            url: String::from("not a URL"),
            ..prepared
        }),
        Err(RuntimeError::Endpoint { .. })
    ));
}

#[test]
fn prepared_call_logging_uses_body_object_and_signed_headers() {
    let authorization = "AWS4-HMAC-SHA256 Credential=access, SignedHeaders=host, Signature=example";
    let prepared = PreparedCall {
        url: String::from("https://runtime.example/invocations"),
        headers: vec![(String::from("Authorization"), authorization.to_owned())],
        body: Vec::new(),
        body_json: IndexMap::from([(String::from("inputs"), Json::Text(String::from("hi")))]),
        region: String::from("us-west-2"),
    };
    let logged = prepared.additional_args();
    assert_eq!(
        logged.api_base.as_deref(),
        Some("https://runtime.example/invocations")
    );
    assert!(matches!(
        logged.complete_input_dict,
        Some(RawBody::Object(body)) if body.get("inputs") == Some(&Json::Text(String::from("hi")))
    ));
    assert_eq!(
        logged.headers.expect("logged headers").get("Authorization"),
        Some(&Json::Text(authorization.to_owned()))
    );
    assert!(logged.passthrough.is_empty());
}

#[tokio::test]
async fn proxied_client_transport_reports_unavailable_upstream() {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("default proxy policy");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("outbound client");
    let outbound = || OutboundRequest {
        method: Method::POST,
        url: Url::parse("http://127.0.0.1:9/invocations").expect("test URL"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_millis(200),
    };
    SagemakerTransport::send(&client, outbound())
        .await
        .expect_err("unavailable upstream");
    assert!(
        SagemakerTransport::open_stream(&client, outbound())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn stream_error_stops_reading_at_the_body_limit() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let polls = Arc::new(AtomicUsize::new(0));
    let source = Arc::clone(&polls);
    let items = stream::unfold(source, |count| async move {
        let index = count.fetch_add(1, Ordering::SeqCst);
        (index < 2).then(|| (Ok(vec![b'x'; 0x0001_0000]), count))
    });
    let chunks: super::ChunkStream = Box::pin(items);
    let error = super::stream_error(503, chunks).await;
    assert_eq!(error.message.len(), 0x0001_0000);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn aws_credential_resolver_adapter_returns_resolved_credentials() {
    let native = native_sources();
    let cache = credential_cache();
    let secrets = SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    };
    let sts = StsClientKwargs {
        verify: Verification::Enabled,
        endpoint_url: None,
        region_name: None,
    };
    let resolver = AwsCredentialResolver {
        cache: &cache,
        native: &native,
        secrets: &secrets,
        sts: &sts,
        max_policy_size: 75,
    };
    let credentials =
        CredentialSource::resolve(&resolver, AwsSettings::default(), String::from("us-west-2"))
            .await
            .expect("resolved static credentials");
    assert_eq!(credentials.access_key_id.expose_secret(), "env-access");
    assert_eq!(credentials.secret_access_key.expose_secret(), "env-secret");
}

fn native_sources() -> NativeSources {
    let static_time = SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH));
    NativeSources {
        environment: BTreeMap::from([
            (
                String::from("AWS_ACCESS_KEY_ID"),
                String::from("env-access"),
            ),
            (
                String::from("AWS_SECRET_ACCESS_KEY"),
                String::from("env-secret"),
            ),
        ]),
        filesystem: Fs::from_slice(&[]),
        client: http_client_fn(|_, _| SharedHttpConnector::new(NoNetwork)),
        clock: static_time,
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(1),
    }
}

fn credential_cache() -> CredentialCache {
    let local = LocalTtlCache::with_clock(Some(1000), None, None, 1024, Arc::new(|| 0.0_f64))
        .expect("local cache");
    CredentialCache::new(DualLocalCache {
        memory: Arc::new(Mutex::new(local)),
        default_in_memory_ttl: None,
        diagnostics: Diagnostics {
            redactor: Arc::new(Redactor::new(16).expect("redactor")),
            emit: Arc::new(|_event| {}),
        },
    })
    .expect("credential cache")
}
