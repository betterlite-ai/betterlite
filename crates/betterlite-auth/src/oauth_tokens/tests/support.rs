use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::oauth_tokens::Settings;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::{self, DefaultGuard};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::{Layer, Registry};

use crate::oauth_tokens::transport::{
    TokenInfoRequest, TokenInfoResponse, TokenInfoTransport, TokenInfoTransportError,
};
use crate::trusted_network::cidr::CidrRanges;
use crate::trusted_network::request::NetworkRequest;
use crate::trusted_network::settings::TrustedNetworkSettings;

pub(super) const ENDPOINT: &str = "https://idp.example/introspect";
pub(super) const TOKEN: &str = "opaque-token-1";
pub(super) const CLIENT_SECRET: &str = "client-secret-value";

pub(super) fn settings(
    endpoint: Option<&str>,
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Settings {
    Settings {
        token_info_endpoint: endpoint.map(str::to_owned),
        user_id_field_name: "sub".to_owned(),
        user_role_field_name: "role".to_owned(),
        user_team_id_field_name: "team_id".to_owned(),
        client_id: client_id.map(str::to_owned),
        client_secret: client_secret.map(|value| SecretString::from(value.to_owned())),
        config_mappings: None,
        enable_oauth2_auth: true,
        enable_oauth2_proxy_auth: false,
    }
}

pub(super) fn token() -> SecretString {
    SecretString::from(TOKEN.to_owned())
}

pub(super) struct Scripted {
    status: u16,
    body: String,
    failure: Option<String>,
    recorded: Mutex<Vec<TokenInfoRequest>>,
}

impl Scripted {
    pub(super) fn answering(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            failure: None,
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing(message: &str) -> Self {
        Self {
            status: 0,
            body: String::new(),
            failure: Some(message.to_owned()),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn take(&self) -> TokenInfoRequest {
        let mut requests = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(requests.len(), 1, "the check sends exactly one request");
        requests.pop().expect("the request was recorded")
    }

    pub(super) fn sent(&self) -> usize {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl TokenInfoTransport for Scripted {
    async fn send(
        &self,
        request: TokenInfoRequest,
    ) -> Result<TokenInfoResponse, TokenInfoTransportError> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if let Some(message) = &self.failure {
            return Err(TokenInfoTransportError::Failed {
                message: message.clone(),
            });
        }
        Ok(TokenInfoResponse {
            status: self.status,
            body: self.body.as_bytes().to_vec(),
        })
    }
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let name = HeaderName::from_bytes(name.as_bytes()).expect("the header name parses");
        let value = HeaderValue::from_str(value).expect("the header value parses");
        headers.insert(name, value);
    }
    headers
}

pub(super) fn mappings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(field, header)| ((*field).to_owned(), (*header).to_owned()))
        .collect()
}

pub(super) fn network<'a>(
    peer: Option<&'a str>,
    forwarded_for: Option<&'a str>,
) -> NetworkRequest<'a> {
    NetworkRequest {
        peer,
        x_forwarded_for: forwarded_for,
        x_forwarded_proto: None,
        host: Some("proxy.example"),
        scheme: "https",
    }
}

pub(super) fn trusted(range: &str) -> TrustedNetworkSettings {
    TrustedNetworkSettings {
        trusted_proxy_ranges: Some(CidrRanges::List(vec![range.to_owned()])),
        ..TrustedNetworkSettings::default()
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<String>>>);

struct RecordedLog(String);

impl Visit for RecordedLog {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: tracing::Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Inner>) {
        let mut recorded = RecordedLog(String::new());
        event.record(&mut recorded);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{} {}", event.metadata().level(), recorded.0));
    }
}

pub(super) fn capture(logs: &Logs) -> DefaultGuard {
    subscriber::set_default(Registry::default().with(logs.clone()))
}

pub(super) fn events(logs: &Logs) -> Vec<String> {
    let recorded = logs.0.lock().unwrap_or_else(PoisonError::into_inner);
    recorded.clone()
}
