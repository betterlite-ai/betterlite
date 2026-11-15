use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::databricks_auth::Headers;
use crate::databricks_auth::endpoint::EndpointType;
use crate::databricks_auth::environment::DatabricksEnv;
use crate::databricks_auth::error::{
    DatabricksError, MISSING_API_BASE, MISSING_API_KEY, SDK_CREDENTIALS,
};
use crate::databricks_auth::oauth::{OauthReply, OauthTokenRequest, OauthTokenSource};
use crate::databricks_auth::validate::{ValidationRequest, validate_environment};
use async_trait::async_trait;
use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

const VERSION: &str = "1.105.1";

struct FakeTokens {
    reply: OauthReply,
    sent: Mutex<Vec<OauthTokenRequest>>,
}

impl FakeTokens {
    fn replying(status: u16, body: &str) -> Self {
        Self {
            reply: OauthReply::Reply {
                status,
                body: SecretString::from(body),
            },
            sent: Mutex::new(Vec::new()),
        }
    }

    fn healthy() -> Self {
        Self::replying(200, r#"{"access_token":"oauth-token"}"#)
    }

    fn sent(&self) -> Vec<OauthTokenRequest> {
        self.sent
            .lock()
            .expect("the sent list is not poisoned")
            .clone()
    }
}

#[async_trait]
impl OauthTokenSource for FakeTokens {
    async fn send(&self, request: &OauthTokenRequest) -> OauthReply {
        self.sent
            .lock()
            .expect("the sent list is not poisoned")
            .push(request.clone());
        self.reply.clone()
    }
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the runtime builds")
}

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn env() -> DatabricksEnv {
    DatabricksEnv {
        client_id: None,
        client_secret: None,
        api_base: None,
        user_agent_override: None,
        version: VERSION.to_owned(),
    }
}

fn oauth_env(base: &str) -> DatabricksEnv {
    DatabricksEnv {
        client_id: Some("client-id".to_owned()),
        client_secret: Some(key("client-secret")),
        api_base: Some(base.to_owned()),
        ..env()
    }
}

fn request<'value>(
    api_key: Option<&'value SecretString>,
    api_base: Option<&'value str>,
) -> ValidationRequest<'value> {
    ValidationRequest {
        api_key,
        api_base,
        endpoint: EndpointType::ChatCompletions,
        custom_endpoint: false,
        headers: None,
        custom_user_agent: None,
    }
}

fn resolve(
    request: ValidationRequest<'_>,
    env: &DatabricksEnv,
    tokens: &FakeTokens,
) -> Result<(String, Headers), DatabricksError> {
    runtime().block_on(validate_environment(request, env, tokens))
}

fn header<'value>(headers: &'value Headers, name: &str) -> Option<&'value str> {
    headers.get(name).map(String::as_str)
}

#[test]
fn oauth_sets_the_bearer_token_and_content_type() {
    let tokens = FakeTokens::healthy();
    let (url, headers) = resolve(request(None, None), &oauth_env("https://host"), &tokens)
        .expect("the oauth branch resolves");
    assert_eq!(url, "https://host/chat/completions");
    assert_eq!(
        header(&headers, "Authorization"),
        Some("Bearer oauth-token")
    );
    assert_eq!(header(&headers, "Content-Type"), Some("application/json"));
    assert_eq!(header(&headers, "User-Agent"), Some("litellm/1.105.1"));
    let sent = tokens.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url, "https://host/oidc/v1/token");
    assert_eq!(
        sent[0].authorization,
        "Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ="
    );
}

#[test]
fn a_key_still_requests_the_token_and_then_wins() {
    let tokens = FakeTokens::healthy();
    let pat = key("pat-key");
    let (_, headers) = resolve(
        request(Some(&pat), None),
        &oauth_env("https://host"),
        &tokens,
    )
    .expect("the oauth branch resolves");
    assert_eq!(header(&headers, "Authorization"), Some("Bearer pat-key"));
    assert_eq!(tokens.sent().len(), 1);
}

#[rstest]
#[case::missing_client_id("", "client-secret")]
#[case::missing_client_secret("client-id", "")]
fn an_empty_oauth_field_skips_the_token_request(#[case] id: &str, #[case] secret: &str) {
    let tokens = FakeTokens::healthy();
    let mut env = oauth_env("https://host");
    env.client_id = Some(id.to_owned());
    env.client_secret = Some(key(secret));
    let mut request = request(None, None);
    request.custom_endpoint = true;
    let error = resolve(request, &env, &tokens).expect_err("no key and a custom endpoint");
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, MISSING_API_KEY);
    assert!(tokens.sent().is_empty());
}

#[test]
fn an_empty_base_skips_the_token_request() {
    let tokens = FakeTokens::healthy();
    let mut request = request(None, Some(""));
    request.custom_endpoint = true;
    let error =
        resolve(request, &oauth_env(""), &tokens).expect_err("no key and a custom endpoint");
    assert_eq!(error.message, MISSING_API_KEY);
    assert!(tokens.sent().is_empty());
}

#[test]
fn a_failed_token_request_fails_the_call() {
    let tokens = FakeTokens::replying(401, "denied");
    let error = resolve(request(None, None), &oauth_env("https://host"), &tokens)
        .expect_err("the token request failed");
    assert_eq!(error.status_code, 401);
}

#[test]
fn a_missing_key_at_a_custom_endpoint_is_the_missing_key_error() {
    let tokens = FakeTokens::healthy();
    let mut request = request(None, Some("https://host"));
    request.custom_endpoint = true;
    let error = resolve(request, &env(), &tokens).expect_err("no key");
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, MISSING_API_KEY);
    assert_eq!(error.headers, None);
}

#[test]
fn a_missing_key_before_the_sdk_fallback_is_the_import_error() {
    let tokens = FakeTokens::healthy();
    let error = resolve(request(None, Some("https://host")), &env(), &tokens)
        .expect_err("no key reaches the sdk path");
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, SDK_CREDENTIALS);
}

#[test]
fn an_empty_headers_map_counts_as_missing() {
    let tokens = FakeTokens::healthy();
    let mut request = request(None, Some("https://host"));
    request.custom_endpoint = true;
    request.headers = Some(BTreeMap::new());
    let error = resolve(request, &env(), &tokens).expect_err("no key");
    assert_eq!(error.message, MISSING_API_KEY);
}

#[test]
fn another_header_skips_the_missing_key_error() {
    let tokens = FakeTokens::healthy();
    let mut request = request(None, Some("https://host"));
    request.headers = Some(BTreeMap::from([("X-Test".to_owned(), "1".to_owned())]));
    let (url, headers) =
        resolve(request, &env(), &tokens).expect("a header counts as a credential");
    assert_eq!(url, "https://host/chat/completions");
    assert_eq!(header(&headers, "Authorization"), None);
    assert_eq!(header(&headers, "X-Test"), Some("1"));
    assert_eq!(header(&headers, "User-Agent"), Some("litellm/1.105.1"));
}

#[test]
fn a_missing_base_at_a_custom_endpoint_is_the_missing_base_error() {
    let tokens = FakeTokens::healthy();
    let pat = key("pat");
    let mut request = request(Some(&pat), None);
    request.custom_endpoint = true;
    let error = resolve(request, &env(), &tokens).expect_err("no base");
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, MISSING_API_BASE);
}

#[test]
fn a_missing_base_before_the_sdk_fallback_is_the_import_error() {
    let tokens = FakeTokens::healthy();
    let pat = key("pat");
    let error = resolve(request(Some(&pat), None), &env(), &tokens)
        .expect_err("no base reaches the sdk path");
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, SDK_CREDENTIALS);
}

#[test]
fn a_key_without_headers_sets_the_bearer_and_content_type() {
    let tokens = FakeTokens::healthy();
    let pat = key("pat");
    let (url, headers) = resolve(request(Some(&pat), Some("https://host")), &env(), &tokens)
        .expect("a key resolves");
    assert_eq!(url, "https://host/chat/completions");
    assert_eq!(header(&headers, "Authorization"), Some("Bearer pat"));
    assert_eq!(header(&headers, "Content-Type"), Some("application/json"));
}

#[test]
fn a_key_replaces_an_authorization_header() {
    let tokens = FakeTokens::healthy();
    let pat = key("pat");
    let mut request = request(Some(&pat), Some("https://host"));
    request.headers = Some(BTreeMap::from([
        ("Authorization".to_owned(), "Bearer old".to_owned()),
        ("X-Keep".to_owned(), "1".to_owned()),
    ]));
    let (_, headers) = resolve(request, &env(), &tokens).expect("a key resolves");
    assert_eq!(header(&headers, "Authorization"), Some("Bearer pat"));
    assert_eq!(header(&headers, "X-Keep"), Some("1"));
}

#[test]
fn the_env_base_is_used_when_the_argument_is_absent() {
    let tokens = FakeTokens::healthy();
    let mut env = env();
    env.api_base = Some("https://env-host".to_owned());
    let pat = key("pat");
    let (url, _) =
        resolve(request(Some(&pat), None), &env, &tokens).expect("the env base resolves");
    assert_eq!(url, "https://env-host/chat/completions");
}

#[test]
fn a_custom_user_agent_prefixes_the_user_agent() {
    let tokens = FakeTokens::healthy();
    let pat = key("pat");
    let mut request = request(Some(&pat), Some("https://host"));
    request.custom_user_agent = Some("acme/9");
    let (_, headers) = resolve(request, &env(), &tokens).expect("a key resolves");
    assert_eq!(header(&headers, "User-Agent"), Some("acme_litellm/1.105.1"));
}

#[test]
fn a_trailing_slash_base_keeps_its_double_slash() {
    let tokens = FakeTokens::healthy();
    let pat = key("pat");
    let (url, _) = resolve(request(Some(&pat), Some("https://host/")), &env(), &tokens)
        .expect("a key resolves");
    assert_eq!(url, "https://host//chat/completions");
}

#[rstest]
#[case::chat(EndpointType::ChatCompletions, false, "https://host/chat/completions")]
#[case::embeddings(EndpointType::Embeddings, false, "https://host/embeddings")]
#[case::chat_custom(EndpointType::ChatCompletions, true, "https://host")]
#[case::embeddings_custom(EndpointType::Embeddings, true, "https://host")]
fn the_endpoint_decides_the_suffix(
    #[case] endpoint: EndpointType,
    #[case] custom_endpoint: bool,
    #[case] expected: &str,
) {
    let tokens = FakeTokens::healthy();
    let pat = key("pat");
    let mut request = request(Some(&pat), Some("https://host"));
    request.endpoint = endpoint;
    request.custom_endpoint = custom_endpoint;
    let (url, _) = resolve(request, &env(), &tokens).expect("a key resolves");
    assert_eq!(url, expected);
}
