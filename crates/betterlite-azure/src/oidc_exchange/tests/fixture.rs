use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_oidc_exchange::Settings;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_client_secret::SecretResolver;
use crate::oidc_exchange::transport::{Clock, HttpReply, Transport};
use crate::oidc_exchange::{AzureOidcExchange, OidcExchangeError, OidcExchangeInput};

pub(super) const NOW: f64 = 1_700_000_000.0;
pub(super) const TENANT: &str = "tenant";
pub(super) const CLIENT: &str = "client";
pub(super) const ASSERTION: &str = "assertion-value";
pub(super) const ASSERTION_ENV: &str = "OIDC_ASSERTION";
pub(super) const ASSERTION_REF: &str = "os.environ/OIDC_ASSERTION";

pub(super) struct Reply {
    status: u16,
    body: String,
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self::status(200, body)
    }

    pub(super) fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) form: Vec<(String, SecretString)>,
}

pub(super) struct FixtureTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Recorded>>,
}

impl FixtureTransport {
    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for FixtureTransport {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<HttpReply> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: url.to_owned(),
                form: form.to_vec(),
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| io::Error::other(format!("no reply for {url}")))?;
        Ok(HttpReply {
            status: reply.status,
            body: SecretString::from(reply.body),
        })
    }
}

pub(super) struct FixtureClock {
    now: Mutex<f64>,
}

impl FixtureClock {
    pub(super) fn advance(&self, seconds: f64) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.mul_add(1.0, seconds);
    }
}

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> f64 {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) struct FixtureResolver {
    environment: BTreeMap<String, String>,
}

// the fixture models B8500's get_secret_str only as far as an os.environ/ reference reads the map
impl SecretResolver for FixtureResolver {
    fn get_secret_str(&self, reference: &str) -> Option<SecretString> {
        let name = reference.strip_prefix("os.environ/")?;
        self.environment
            .get(name)
            .map(|value| SecretString::from(value.as_str()))
    }
}

pub(super) struct Fixture {
    pub(super) transport: Arc<FixtureTransport>,
    pub(super) clock: Arc<FixtureClock>,
    pub(super) environment: BTreeMap<String, String>,
}

impl Fixture {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            transport: Arc::new(FixtureTransport {
                replies: Mutex::new(VecDeque::from(replies)),
                requests: Mutex::new(Vec::new()),
            }),
            clock: Arc::new(FixtureClock {
                now: Mutex::new(NOW),
            }),
            environment: BTreeMap::from([(ASSERTION_ENV.to_owned(), ASSERTION.to_owned())]),
        }
    }

    pub(super) fn with_environment(mut self, pairs: &[(&str, &str)]) -> Self {
        self.environment.extend(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );
        self
    }

    pub(super) fn settings(&self) -> Settings {
        ConfigLoader {
            lookup: self.environment.clone(),
        }
        .load("")
        .unwrap()
        .sections
        .azure_oidc_exchange
    }

    pub(super) fn exchange(&self) -> AzureOidcExchange {
        let transport = Arc::clone(&self.transport);
        let clock = Arc::clone(&self.clock);
        AzureOidcExchange::new(self.settings(), self.environment.clone(), transport, clock).unwrap()
    }

    pub(super) fn resolver(&self) -> FixtureResolver {
        FixtureResolver {
            environment: self.environment.clone(),
        }
    }

    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.transport.recorded()
    }

    pub(super) fn error_of(&self, input: &OidcExchangeInput) -> OidcExchangeError {
        self.exchange().token(input, &self.resolver()).unwrap_err()
    }
}

pub(super) fn input(
    token: &str,
    client: Option<&str>,
    tenant: Option<&str>,
    scope: Option<&str>,
) -> OidcExchangeInput {
    OidcExchangeInput {
        azure_ad_token: SecretString::from(token),
        azure_client_id: client.map(str::to_owned),
        azure_tenant_id: tenant.map(str::to_owned),
        scope: scope.map(str::to_owned),
    }
}

pub(super) fn default_input() -> OidcExchangeInput {
    input(ASSERTION_REF, Some(CLIENT), Some(TENANT), None)
}

pub(super) fn form_map(request: &Recorded) -> BTreeMap<String, String> {
    request
        .form
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

pub(super) fn form_value(request: &Recorded, name: &str) -> Option<String> {
    form_map(request).get(name).cloned()
}
