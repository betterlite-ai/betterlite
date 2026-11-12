#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::openai_credentials::Settings as CredentialSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::task::yield_now;
use url::Url;

use super::super::endpoint::{
    ModelListError, ModelListRequest, ModelListTransport, complete_url, get_models,
    validate_environment,
};
use crate::credentials::{ExplicitCredentials, LiteLlmGlobals, get_openai_credentials};
use crate::openai_base_config::Headers;

struct Script {
    status: u16,
    body: String,
    requests: Mutex<Vec<(String, Option<String>)>>,
}

impl Script {
    fn replying(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requested(&self) -> Vec<(String, Option<String>)> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelListTransport for Script {
    async fn get(
        &self,
        url: &Url,
        headers: &HeaderMap,
        _deadline: Duration,
    ) -> Result<UpstreamResponse, TransportError> {
        yield_now().await;
        let authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.requests
            .lock()
            .unwrap()
            .push((url.to_string(), authorization));
        Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone().into_bytes(),
        })
    }
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn fetch(script: &Script, request: &ModelListRequest<'_>) -> Result<Vec<String>, ModelListError> {
    block_on(get_models(script, request))
}

fn request<'base>(api_base: Option<&'base str>, api_key: Option<&str>) -> ModelListRequest<'base> {
    ModelListRequest {
        api_key: api_key.map(SecretString::from),
        api_base,
        environment_api_key: None,
        deadline: Duration::from_secs(600),
    }
}

const MODELS: &str = r#"{"data": [{"id": "b"}, {"id": "a"}]}"#;

pub(super) fn model_list() {
    let script = Script::replying(200, MODELS);
    let models = fetch(
        &script,
        &request(Some("https://Example.COM:8443/v1/path?q=1"), Some("k")),
    )
    .unwrap();
    assert_eq!(models, ["b", "a"]);
    assert_eq!(
        script.requested(),
        [(
            String::from("https://example.com:8443/v1/models"),
            Some(String::from("Bearer k"))
        )]
    );
    default_base();
    proxied_client_request();
}

async fn serve_models(listener: TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !received.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.unwrap();
        received.extend_from_slice(&chunk[..read]);
    }
    let length = MODELS.len();
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {length}\r\nconnection: close\r\n\r\n{MODELS}"
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    String::from_utf8(received).unwrap()
}

fn proxied_client_request() {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let (head, models) = runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/ignored", listener.local_addr().unwrap());
        let models_request = request(Some(base.as_str()), Some("k"));
        tokio::join!(serve_models(listener), get_models(&client, &models_request))
    });
    assert_eq!(models.unwrap(), ["b", "a"]);
    assert!(head.starts_with("GET /v1/models HTTP/1.1"), "{head}");
    assert!(head.contains("authorization: Bearer k"), "{head}");
}

fn default_base() {
    let script = Script::replying(200, MODELS);
    let mut default_request = request(None, None);
    default_request.environment_api_key = Some(SecretString::from("env-key"));
    assert_eq!(fetch(&script, &default_request).unwrap(), ["b", "a"]);
    assert_eq!(
        script.requested(),
        [(
            String::from("https://api.openai.com/v1/models"),
            Some(String::from("Bearer env-key"))
        )]
    );

    let no_key = Script::replying(200, MODELS);
    drop(fetch(&no_key, &request(None, None)).unwrap());
    assert_eq!(no_key.requested()[0].1.as_deref(), Some("Bearer None"));

    let explicit = Script::replying(200, MODELS);
    let mut explicit_request = request(None, Some("explicit"));
    explicit_request.environment_api_key = Some(SecretString::from("env-key"));
    drop(fetch(&explicit, &explicit_request).unwrap());
    assert_eq!(
        explicit.requested()[0].1.as_deref(),
        Some("Bearer explicit")
    );
}

pub(super) fn model_list_decode_failure() {
    for body in [
        "not json",
        r#"{"models": []}"#,
        r#"{"data": "x"}"#,
        r#"{"data": [{}]}"#,
        r#"{"data": [{"id": 5}]}"#,
    ] {
        let error = fetch(&Script::replying(200, body), &request(None, Some("k"))).unwrap_err();
        assert!(matches!(error, ModelListError::Decode { .. }), "{body}");
    }
    let failed = fetch(
        &Script::replying(500, r#"{"error": "boom"}"#),
        &request(None, Some("k")),
    )
    .unwrap_err();
    assert!(matches!(failed, ModelListError::Status { status: 500 }));
    let invalid = fetch(
        &Script::replying(200, MODELS),
        &request(Some("not a url"), Some("k")),
    )
    .unwrap_err();
    assert!(matches!(invalid, ModelListError::InvalidBase { .. }));
}

pub(super) fn query_url_join() {
    assert_eq!(
        complete_url(None),
        "https://api.openai.com/chat/completions"
    );
    assert_eq!(
        complete_url(Some("https://h/v1/")),
        "https://h/v1/chat/completions"
    );
    assert_eq!(
        complete_url(Some("https://h/v1//")),
        "https://h/v1/chat/completions"
    );
    assert_eq!(
        complete_url(Some("https://h/v1/chat/completions")),
        "https://h/v1/chat/completions"
    );
    assert_eq!(
        complete_url(Some("https://h/v1/chat/completions/")),
        "https://h/v1/chat/completions"
    );
    assert_eq!(
        complete_url(Some("https://h/v1?x=1")),
        "https://h/v1?x=1/chat/completions"
    );
    assert_eq!(
        complete_url(Some("https://h/chat/completions-proxy")),
        "https://h/chat/completions-proxy"
    );
    assert_eq!(complete_url(Some("")), "/chat/completions");
}

pub(super) fn chat_auth_content_type() {
    let key = SecretString::from("k");
    let with_key = validate_environment(Some(&key), &Headers::new());
    assert_eq!(
        with_key.get("Authorization").map(String::as_str),
        Some("Bearer k")
    );
    assert_eq!(
        with_key.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    kept_content_type(&key);
    none_key_leaves_authorization();
    overwritten_authorization(&key);
}

fn kept_content_type(key: &SecretString) {
    let lower = Headers::from([(String::from("content-type"), String::from("text/plain"))]);
    let kept = validate_environment(Some(key), &lower);
    assert_eq!(
        kept.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert!(!kept.contains_key("Content-Type"));
    let exact = Headers::from([(String::from("Content-Type"), String::from("text/plain"))]);
    assert_eq!(
        validate_environment(Some(key), &exact)
            .get("Content-Type")
            .map(String::as_str),
        Some("text/plain")
    );
    let odd = Headers::from([(String::from("Content-type"), String::from("text/plain"))]);
    let added = validate_environment(None, &odd);
    assert_eq!(
        added.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

fn none_key_leaves_authorization() {
    let caller = Headers::from([(String::from("Authorization"), String::from("caller"))]);
    let untouched = validate_environment(None, &caller);
    assert_eq!(
        untouched.get("Authorization").map(String::as_str),
        Some("caller")
    );
    assert_eq!(
        untouched.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    let empty = validate_environment(None, &Headers::new());
    assert!(!empty.contains_key("Authorization"));
}

fn overwritten_authorization(key: &SecretString) {
    let caller = Headers::from([(String::from("Authorization"), String::from("caller"))]);
    let overwritten = validate_environment(Some(key), &caller);
    assert_eq!(
        overwritten.get("Authorization").map(String::as_str),
        Some("Bearer k")
    );
}

fn credential_settings(api_base: &str) -> CredentialSettings {
    CredentialSettings {
        api_base: api_base.to_owned(),
        organization: None,
    }
}

fn resolved_key(
    api_key: Option<SecretString>,
    litellm_api_key: Option<SecretString>,
    openai_key: Option<SecretString>,
    environment: Option<SecretString>,
) -> Option<String> {
    get_openai_credentials(
        ExplicitCredentials {
            api_key,
            api_base: None,
            organization: None,
        },
        LiteLlmGlobals {
            api_base: None,
            api_key: litellm_api_key,
            openai_key,
            organization: None,
        },
        &credential_settings("https://env.example/v1"),
        environment,
    )
    .api_key
    .as_ref()
    .map(|key| key.expose_secret().to_owned())
}

fn credential_precedence() {
    assert_eq!(
        resolved_key(
            Some(SecretString::from("explicit-key")),
            Some(SecretString::from("global-key")),
            Some(SecretString::from("openai-key")),
            Some(SecretString::from("env-key")),
        ),
        Some(String::from("explicit-key"))
    );
    assert_eq!(
        resolved_key(
            None,
            Some(SecretString::from("global-key")),
            Some(SecretString::from("openai-key")),
            Some(SecretString::from("env-key")),
        ),
        Some(String::from("global-key"))
    );
    assert_eq!(
        resolved_key(
            None,
            None,
            Some(SecretString::from("openai-key")),
            Some(SecretString::from("env-key"))
        ),
        Some(String::from("openai-key"))
    );
    assert_eq!(
        resolved_key(
            Some(SecretString::from("")),
            None,
            None,
            Some(SecretString::from("env-key"))
        ),
        Some(String::from("env-key"))
    );
}

fn resolved_base(explicit: Option<&str>, litellm: Option<&str>) -> String {
    get_openai_credentials(
        ExplicitCredentials {
            api_base: explicit.map(str::to_owned),
            api_key: None,
            organization: None,
        },
        LiteLlmGlobals {
            api_base: litellm.map(str::to_owned),
            api_key: None,
            openai_key: None,
            organization: None,
        },
        &credential_settings("https://env.example/v1"),
        None,
    )
    .api_base
}

fn credential_bases() {
    assert_eq!(
        resolved_base(
            Some("https://explicit.example/v1"),
            Some("https://litellm.example/v1")
        ),
        "https://explicit.example/v1"
    );
    assert_eq!(
        resolved_base(None, Some("https://litellm.example/v1")),
        "https://litellm.example/v1"
    );
    assert_eq!(resolved_base(None, None), "https://env.example/v1");
    assert_eq!(
        resolved_key(None, None, None, None),
        None,
        "no key anywhere resolves to none"
    );
}

pub(super) fn credential_accessor_defaults() {
    credential_precedence();
    credential_bases();
}
