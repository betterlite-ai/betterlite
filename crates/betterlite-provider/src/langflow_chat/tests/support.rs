use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_tokenizer::count::error::CountError;
use secrecy::SecretString;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::langflow_chat::reply::TokenCounter;

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the fixture has no OIDC network"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the fixture has no Azure token provider"),
        })
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

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn object(wire: &str) -> Object {
    serde_json::from_str(wire).expect("the fixture object is JSON")
}

pub(super) fn messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).expect("the fixture messages are JSON")
}

pub(super) fn key(value: &str) -> SecretString {
    SecretString::from(String::from(value))
}

pub(super) struct FixedCounter {
    pub(super) prompt: Option<u64>,
    pub(super) completion: Option<u64>,
    pub(super) models: Mutex<Vec<String>>,
}

impl FixedCounter {
    pub(super) fn new(prompt: Option<u64>, completion: Option<u64>) -> Self {
        Self {
            prompt,
            completion,
            models: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn models(&self) -> Vec<String> {
        self.models
            .lock()
            .expect("the fixture lock is held")
            .clone()
    }
}

#[async_trait]
impl TokenCounter for FixedCounter {
    async fn count_messages(&self, model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        self.models
            .lock()
            .expect("the fixture lock is held")
            .push(String::from(model));
        self.prompt.ok_or(CountError::MissingInput)
    }

    async fn count_response_tokens(&self, model: &str, _text: &str) -> Result<u64, CountError> {
        self.models
            .lock()
            .expect("the fixture lock is held")
            .push(String::from(model));
        self.completion.ok_or(CountError::MissingInput)
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("the fixture lock is held").clone())
            .expect("the captured log is UTF-8")
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

pub(super) fn reply_body(content: &str) -> String {
    let prefix = r#"{"outputs":[{"outputs":[{"results":{"message":{"text":""#;
    let suffix = r#""}}}]}]}"#;
    [prefix, content, suffix].concat()
}

pub(super) fn empty_object() -> Object {
    Object::new()
}
