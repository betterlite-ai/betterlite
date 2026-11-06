use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::chatgpt::chat::{Authenticator, ChatGptConfig, ChatGptError};
use betterlite_openai::chatgpt::headers::session::SessionIdentity;
use betterlite_openai::chatgpt::login::error::LoginError;
use betterlite_openai::chatgpt::login::http::{AuthPost, AuthResponse, AuthTransport, HttpFailure};
use betterlite_openai::chatgpt::login::{ChatGptLogin, DeviceDisplay, Pause};
use betterlite_openai::chatgpt::token_store::codec::AuthRecord;
use betterlite_openai::chatgpt::token_store::files::{FileAccess, StoreEvent};
use betterlite_openai::chatgpt::token_store::{ChatGptTokenStore, Clock, StoreError};
use betterlite_openai::openai_base_config::{ConfigFamily, Headers, OpenAiConfig, OpenAiSettings};
use rust_decimal::Decimal;
use secrecy::SecretString;

use super::Input;

const SESSION_ID: &str = "00000000-0000-4000-8000-000000000000";

pub(super) struct FixedSession;

impl SessionIdentity for FixedSession {
    fn generate(&self) -> String {
        SESSION_ID.to_owned()
    }
}

pub(super) struct MemoryFiles {
    content: Option<Vec<u8>>,
}

impl FileAccess for MemoryFiles {
    fn ensure_dir(&mut self, _path: &Path) -> io::Result<()> {
        Ok(())
    }

    fn read(&mut self, _path: &Path) -> io::Result<Vec<u8>> {
        self.content
            .clone()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn write(&mut self, _path: &Path, _record: &AuthRecord) -> io::Result<()> {
        Ok(())
    }

    fn report(&mut self, _event: StoreEvent) {}
}

pub(super) struct FixedClock {
    now: Decimal,
}

impl Clock for FixedClock {
    fn now(&self) -> Decimal {
        self.now
    }

    fn sleep(&mut self, _duration: Duration) {}
}

impl Pause for FixedClock {
    async fn pause(&mut self, _duration: Duration) {}
}

pub(super) struct NoTransport;

impl AuthTransport for NoTransport {
    fn post(
        &mut self,
        _request: AuthPost<'_>,
    ) -> impl Future<Output = Result<AuthResponse, HttpFailure>> + Send {
        ready(Err(HttpFailure::Transport(TransportError::Closed)))
    }
}

pub(super) struct NoDisplay;

impl DeviceDisplay for NoDisplay {
    fn show(&mut self, _verify_url: &str, _user_code: &SecretString) {}
}

pub(super) struct Recording<L> {
    pub(super) login: L,
    pub(super) calls: Mutex<Vec<&'static str>>,
}

impl<L: Authenticator + Send> Authenticator for Recording<L> {
    fn api_base(&self) -> &str {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("get_api_base");
        self.login.api_base()
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, LoginError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("get_access_token");
        self.login.access_token()
    }

    fn account_id(&mut self) -> Option<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("get_account_id");
        self.login.account_id()
    }
}

pub(super) type Login = ChatGptLogin<MemoryFiles, FixedClock, NoTransport, NoDisplay>;

#[derive(Debug, thiserror::Error)]
pub(super) enum FixtureError {
    #[error("fixture configuration failed")]
    Config(#[from] ConfigError),
    #[error("fixture token store failed")]
    Store(#[from] StoreError),
    #[error("fixture JSON failed")]
    Json(#[from] serde_json::Error),
    #[error("fixture runtime failed")]
    Runtime(#[from] io::Error),
    #[error("ChatGPT preparation failed")]
    Chat(#[from] ChatGptError),
}

pub(super) fn assemble(
    input: &Input,
    api_key: Option<SecretString>,
    headers: Headers,
) -> Result<ChatGptConfig<Recording<Login>>, FixtureError> {
    let sections = ConfigLoader {
        lookup: input.environment.clone(),
    }
    .load("")?;
    let store = ChatGptTokenStore::new(
        sections.sections.chatgpt_token_store,
        MemoryFiles {
            content: input.record.clone().map(String::into_bytes),
        },
        FixedClock { now: input.now },
    )?;
    Ok(ChatGptConfig {
        base: OpenAiConfig {
            family: ConfigFamily::Other,
            api_key,
            headers,
            settings: OpenAiSettings::default(),
        },
        authenticator: Recording {
            login: ChatGptLogin {
                store,
                http: NoTransport,
                display: NoDisplay,
            },
            calls: Mutex::new(Vec::new()),
        },
        header_settings: sections.sections.chatgpt_headers,
    })
}
