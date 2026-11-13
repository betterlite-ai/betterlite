use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::auth_config::AuthConfig;
use betterlite_mcp::credential_contracts::auth_config::oauth::{
    DEFAULT_SUBJECT_TOKEN_TYPE, TokenExchangeConfig, TokenExchangeProfile,
};
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::server_spec::ServerSpec;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::yield_now;

use super::super::super::refresh_store::InProcessRefreshCoordinator;
use super::super::super::{
    InMemoryTokenCacheBackend, OAuthToken, TokenCacheEntries, TokenKey, WallClock,
};
use super::super::{
    DEFAULT_TTL, EXPIRY_BUFFER, ExchangeBody, ExchangeHttpPost, ExchangeRejection, MIN_TTL,
    OboTokenExchanger,
};

pub(super) struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    pub(super) fn new(seconds: i64) -> Self {
        Self(Mutex::new(at(seconds)))
    }

    pub(super) fn set(&self, seconds: i64) {
        *self.0.lock().unwrap() = at(seconds);
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

pub(super) fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

pub(super) fn clock(seconds: i64) -> Arc<ManualClock> {
    Arc::new(ManualClock::new(seconds))
}

#[derive(Clone)]
pub(super) enum Scripted {
    Body(ExchangeBody),
    Transport,
    SubjectRejected {
        detail: String,
        claims: Option<String>,
    },
    ClientFault {
        code: String,
    },
}

#[derive(Clone)]
pub(super) struct RecordedCall {
    pub(super) url: String,
    pub(super) form: BTreeMap<String, String>,
    pub(super) headers: BTreeMap<String, String>,
}

pub(super) struct RecordingPost {
    responses: Mutex<VecDeque<Scripted>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl RecordingPost {
    pub(super) fn new(responses: Vec<Scripted>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ExchangeHttpPost for RecordingPost {
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, SecretString>,
        headers: &BTreeMap<String, SecretString>,
    ) -> Result<Option<ExchangeBody>, ExchangeRejection> {
        let recorded = RecordedCall {
            url: url.to_owned(),
            form: form
                .iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect(),
            headers: headers
                .iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect(),
        };
        self.calls.lock().unwrap().push(recorded);
        let next = self.responses.lock().unwrap().pop_front();
        yield_now().await;
        match next.unwrap_or(Scripted::Transport) {
            Scripted::Body(body) => Ok(Some(body)),
            Scripted::Transport => Ok(None),
            Scripted::SubjectRejected { detail, claims } => {
                Err(ExchangeRejection::SubjectRejected { detail, claims })
            }
            Scripted::ClientFault { code } => Err(ExchangeRejection::ClientFault { code }),
        }
    }
}

pub(super) type TestExchanger = OboTokenExchanger<
    RecordingPost,
    InMemoryTokenCacheBackend,
    InProcessRefreshCoordinator<Option<OAuthToken>, ExchangeRejection>,
>;

pub(super) fn exchanger(post: RecordingPost, clock: Arc<ManualClock>) -> TestExchanger {
    let manual_clock: Arc<ManualClock> = Arc::clone(&clock);
    let cache_clock: Arc<dyn WallClock> = manual_clock;
    OboTokenExchanger {
        post,
        cache: InMemoryTokenCacheBackend {
            max_size: 16,
            clock: cache_clock,
            entries: TokenCacheEntries::default(),
        },
        coordinator: InProcessRefreshCoordinator::default(),
        clock,
        default_ttl: DEFAULT_TTL,
        min_ttl: MIN_TTL,
        expiry_buffer: EXPIRY_BUFFER,
    }
}

pub(super) fn body(raw: &str) -> ExchangeBody {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn pairs(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn base_config() -> TokenExchangeConfig {
    TokenExchangeConfig {
        header: HeaderCarrier::default(),
        profile: TokenExchangeProfile::Rfc8693,
        subject_token_type: DEFAULT_SUBJECT_TOKEN_TYPE.to_owned(),
        token_exchange_endpoint: Some("https://idp.example.com/token".to_owned()),
        audience: None,
        client_id: Some("cid".to_owned()),
        client_secret: Some(SecretString::from("csec")),
        token_endpoint_auth_method: None,
        scopes: Vec::new(),
    }
}

pub(super) fn config() -> TokenExchangeConfig {
    let mut config = base_config();
    config.audience = Some("https://up.example.com".to_owned());
    config.scopes = vec!["s1".to_owned(), "s2".to_owned()];
    config
}

pub(super) fn server() -> ServerSpec {
    ServerSpec {
        server_id: "srv".to_owned(),
        resource: "https://up.example.com".to_owned(),
        config: AuthConfig::TokenExchange(config()),
    }
}

pub(super) fn server_named(server_id: &str) -> ServerSpec {
    ServerSpec {
        server_id: server_id.to_owned(),
        resource: "https://up.example.com".to_owned(),
        config: AuthConfig::TokenExchange(config()),
    }
}
