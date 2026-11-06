#![cfg(test)]

use std::collections::BTreeSet;
use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_config::sections::chatgpt_headers::Settings as HeaderSettings;
use betterlite_config::sections::chatgpt_token_store::Settings as TokenStoreSettings;
use betterlite_http::proxy::transport_error::TransportError;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};

use super::{Authenticator, ChatGptConfig, ChatGptError};
use crate::chatgpt::headers::PlatformEnvironment;
use crate::chatgpt::headers::session::{LitellmParamsInput, SessionIdentity};
use crate::chatgpt::login::error::{Failure, LoginError};
use crate::chatgpt::login::http::{AuthPost, AuthResponse, AuthTransport, HttpFailure};
use crate::chatgpt::login::{ChatGptLogin, DeviceDisplay, Pause};
use crate::chatgpt::token_store::codec::AuthRecord;
use crate::chatgpt::token_store::files::FileAccess;
use crate::chatgpt::token_store::{ChatGptTokenStore, Clock};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

struct FakeAuthenticator {
    base: String,
    token: String,
    account_id: Option<String>,
    failure: Option<LoginError>,
    calls: Vec<&'static str>,
}

impl Authenticator for FakeAuthenticator {
    fn api_base(&self) -> &str {
        &self.base
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, LoginError>> + Send {
        self.calls.push("get_access_token");
        let outcome = match self.failure.take() {
            Some(error) => Err(error),
            None => Ok(SecretString::from(self.token.clone())),
        };
        ready(outcome)
    }

    fn account_id(&mut self) -> Option<String> {
        self.calls.push("get_account_id");
        self.account_id.clone()
    }
}

struct FakePlatform;

impl PlatformEnvironment for FakePlatform {
    fn os_type(&self) -> String {
        "Linux".to_owned()
    }

    fn os_release(&self) -> String {
        "6.1.0".to_owned()
    }

    fn arch(&self) -> String {
        "x86_64".to_owned()
    }
}

struct FixedSession;

impl SessionIdentity for FixedSession {
    fn generate(&self) -> String {
        "generated-session".to_owned()
    }
}

fn authenticator() -> FakeAuthenticator {
    FakeAuthenticator {
        base: "https://configured.example/backend".to_owned(),
        token: "stored-token".to_owned(),
        account_id: Some("account-9".to_owned()),
        failure: None,
        calls: Vec::new(),
    }
}

fn config(api_key: Option<&str>, headers: Headers) -> ChatGptConfig<FakeAuthenticator> {
    ChatGptConfig {
        base: OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: api_key.map(SecretString::from),
            headers,
            settings: OpenAiSettings::default(),
        },
        authenticator: authenticator(),
        header_settings: HeaderSettings::default(),
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::from([String::from("gpt-4o")]),
        text_models: BTreeSet::new(),
    }
}

fn stream_map(value: Value) -> Map<String, Value> {
    Map::from_iter([(String::from("stream"), value)])
}

struct CountingFiles {
    reads: Arc<AtomicUsize>,
}

impl FileAccess for CountingFiles {
    fn ensure_dir(&mut self, _path: &Path) -> io::Result<()> {
        Ok(())
    }

    fn read(&mut self, _path: &Path) -> io::Result<Vec<u8>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    fn write(&mut self, _path: &Path, _record: &AuthRecord) -> io::Result<()> {
        Ok(())
    }
}

struct NoClock;

impl Clock for NoClock {
    fn now(&self) -> Decimal {
        Decimal::ZERO
    }

    fn sleep(&mut self, _duration: Duration) {}
}

impl Pause for NoClock {
    async fn pause(&mut self, _duration: Duration) {}
}

struct NoTransport;

impl AuthTransport for NoTransport {
    fn post(
        &mut self,
        _request: AuthPost<'_>,
    ) -> impl Future<Output = Result<AuthResponse, HttpFailure>> + Send {
        ready(Err(HttpFailure::Transport(TransportError::Closed)))
    }
}

struct NoDisplay;

impl DeviceDisplay for NoDisplay {
    fn show(&mut self, _verify_url: &str, _user_code: &SecretString) {}
}

#[test]
fn api_base_comes_from_the_real_login_without_a_token_lookup() {
    let reads = Arc::new(AtomicUsize::new(0));
    let settings = TokenStoreSettings {
        token_dir: String::from("/tokens"),
        auth_file: String::from("auth.json"),
        api_base: String::from("https://configured.example/backend"),
    };
    let store = ChatGptTokenStore::new(
        settings,
        CountingFiles {
            reads: Arc::clone(&reads),
        },
        NoClock,
    )
    .expect("the store accepts its token directory");
    let config = ChatGptConfig {
        base: OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        },
        authenticator: ChatGptLogin {
            store,
            http: NoTransport,
            display: NoDisplay,
        },
        header_settings: HeaderSettings::default(),
    };
    assert_eq!(
        config.authenticator.api_base(),
        "https://configured.example/backend"
    );
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn provider_info_ignores_the_supplied_api_base_and_key() {
    let mut config = config(None, Headers::new());
    let info = config
        .provider_info(
            "gpt-5",
            Some("https://ignored.example"),
            Some("sk-ignored"),
            "chatgpt",
        )
        .await
        .expect("the stored token is valid");
    assert_eq!(info.api_base, "https://configured.example/backend");
    assert_eq!(info.api_key.expose_secret(), "stored-token");
    assert_eq!(info.provider, "chatgpt");
    assert_eq!(config.authenticator.calls, ["get_access_token"]);
}

#[tokio::test]
async fn provider_info_translates_access_token_failures_to_authentication() {
    let mut subject = config(None, Headers::new());
    subject.authenticator.failure = Some(LoginError::AccessToken(Failure::Status(500)));
    let outcome = subject.provider_info("gpt-5", None, None, "chatgpt").await;
    let Err(error) = outcome else {
        panic!("an access token failure must fail the provider info");
    };
    assert!(matches!(&error, ChatGptError::Authentication { .. }));
    assert_eq!(error.status(), 401);
}

#[tokio::test]
async fn provider_info_keeps_device_code_failures_as_login_errors() {
    let mut subject = config(None, Headers::new());
    subject.authenticator.failure = Some(LoginError::DeviceCode(Failure::Status(503)));
    let outcome = subject.provider_info("gpt-5", None, None, "chatgpt").await;
    let Err(error) = outcome else {
        panic!("a device code failure must fail the provider info");
    };
    assert!(matches!(&error, ChatGptError::Login { .. }));
    assert_eq!(error.status(), 503);
}

#[test]
fn validate_environment_overlays_parent_headers_on_chatgpt_defaults() {
    let headers = Headers::from([
        ("Authorization".to_owned(), "Bearer caller".to_owned()),
        ("x-caller".to_owned(), "kept".to_owned()),
    ]);
    let mut config = config(Some("stored-token"), headers);
    let params = LitellmParamsInput::Mapping(Map::from_iter([(
        String::from("litellm_session_id"),
        json!("session-7"),
    )]));
    let validated = config.validate_environment(&params, &FakePlatform, &FixedSession);
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer caller")
    );
    assert_eq!(validated.get("x-caller").map(String::as_str), Some("kept"));
    assert_eq!(
        validated.get("session_id").map(String::as_str),
        Some("session-7")
    );
    assert_eq!(
        validated.get("ChatGPT-Account-Id").map(String::as_str),
        Some("account-9")
    );
    assert_eq!(
        validated.get("originator").map(String::as_str),
        Some("codex_cli_rs")
    );
    assert_eq!(
        validated.get("content-type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn validate_environment_generates_a_session_id_when_none_carries_one() {
    let mut config = config(Some("stored-token"), Headers::new());
    let headers =
        config.validate_environment(&LitellmParamsInput::Absent, &FakePlatform, &FixedSession);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer stored-token")
    );
    assert_eq!(
        headers.get("session_id").map(String::as_str),
        Some("generated-session")
    );
}

#[test]
fn validate_environment_authorizes_with_bearer_none_without_a_key() {
    let mut config = config(None, Headers::new());
    let headers =
        config.validate_environment(&LitellmParamsInput::Absent, &FakePlatform, &FixedSession);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer None")
    );
}

#[rstest]
#[case::absent(Map::new(), stream_map(Value::Bool(false)))]
#[case::explicit_true(stream_map(Value::Bool(true)), stream_map(Value::Bool(true)))]
#[case::explicit_null(stream_map(Value::Null), stream_map(Value::Null))]
fn map_openai_params_sets_stream_false_only_when_absent(
    #[case] requested: Map<String, Value>,
    #[case] expected: Map<String, Value>,
) {
    let config = config(None, Headers::new());
    let mut optional = Map::new();
    config.map_openai_params("gpt-4o", &requested, &mut optional, &catalog());
    assert_eq!(optional, expected);
}

#[test]
fn map_openai_params_keeps_a_stream_set_before_the_mapping() {
    let config = config(None, Headers::new());
    let mut optional = Map::from_iter([(String::from("stream"), Value::Bool(true))]);
    config.map_openai_params("gpt-4o", &Map::new(), &mut optional, &catalog());
    assert_eq!(optional.get("stream"), Some(&Value::Bool(true)));
}
