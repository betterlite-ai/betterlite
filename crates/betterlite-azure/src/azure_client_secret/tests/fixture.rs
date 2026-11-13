use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::azure_client_secret::Settings;
use secrecy::SecretString;

use crate::azure_client_secret::provider::{ClientSecretProvider, ProviderCache};
use crate::azure_client_secret::transport::{Clock, HttpMethod, HttpReply, HttpRequest, Transport};
use crate::azure_client_secret::{ClientSecretError, ClientSecretInput, SecretResolver};

pub(super) const HOST: &str = "login.microsoftonline.com";
pub(super) const TENANT: &str = "tenant";
pub(super) const CLIENT: &str = "client";
pub(super) const SECRET: &str = "secret";
pub(super) const NOW: i64 = 1_700_000_000;

#[derive(Clone)]
pub(super) struct Reply {
    pub(super) url_contains: Option<String>,
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) error: Option<String>,
}

impl Reply {
    pub(super) fn ok(url_contains: &str, body: &str) -> Self {
        Self::status(200, url_contains, body)
    }

    pub(super) fn status(status: u16, url_contains: &str, body: &str) -> Self {
        Self {
            url_contains: Some(url_contains.to_owned()),
            status,
            body: body.to_owned(),
            error: None,
        }
    }

    pub(super) fn failure(url_contains: &str) -> Self {
        Self {
            url_contains: Some(url_contains.to_owned()),
            status: 500,
            body: String::new(),
            error: Some("connection reset".to_owned()),
        }
    }
}

pub(super) fn openid(host: &str, tenant: &str) -> Reply {
    Reply::ok(
        "well-known/openid-configuration",
        &format!(
            r#"{{"issuer":"https://{host}/{tenant}/v2.0","authorization_endpoint":"https://{host}/{tenant}/oauth2/v2.0/authorize","token_endpoint":"https://{host}/{tenant}/oauth2/v2.0/token"}}"#
        ),
    )
}

pub(super) fn openid_for(host: &str) -> Reply {
    openid(host, TENANT)
}

pub(super) fn token(body: &str) -> Reply {
    Reply::ok("/oauth2/v2.0/token", body)
}

pub(super) fn instance_metadata(payload: &str) -> Reply {
    Reply::ok("/common/discovery/instance", payload)
}

pub(super) fn imds(body: &str) -> Reply {
    Reply::ok("169.254.169.254", body)
}

pub(super) struct FixtureTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FixtureTransport {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn requests_matching(&self, needle: &str) -> Vec<HttpRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.url.contains(needle))
            .collect()
    }

    pub(super) fn posts(&self) -> Vec<HttpRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.method == HttpMethod::Post)
            .collect()
    }
}

impl Transport for FixtureTransport {
    fn send(&self, request: &HttpRequest) -> io::Result<HttpReply> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let reply = {
            let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
            let index = replies
                .iter()
                .position(|reply| {
                    reply
                        .url_contains
                        .as_ref()
                        .is_none_or(|needle| request.url.contains(needle))
                })
                .ok_or_else(|| io::Error::other(format!("no reply for {}", request.url)))?;
            replies
                .remove(index)
                .ok_or_else(|| io::Error::other("reply disappeared"))?
        };
        match reply.error {
            Some(message) => Err(io::Error::other(message)),
            None => Ok(HttpReply {
                status: reply.status,
                body: SecretString::from(reply.body),
            }),
        }
    }
}

pub(super) struct FixtureClock {
    now: AtomicI64,
}

impl FixtureClock {
    pub(super) fn new(now: i64) -> Self {
        Self {
            now: AtomicI64::new(now),
        }
    }

    pub(super) fn set(&self, now: i64) {
        self.now.store(now, Ordering::SeqCst);
    }
}

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

pub(super) struct FixtureResolver {
    pub(super) environment: BTreeMap<String, String>,
}

impl FixtureResolver {
    pub(super) fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            environment: pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }
}

// secrets resolve like B8500's get_secret_str: an os.environ/ name reads the map, and true/false read back as unset
impl SecretResolver for FixtureResolver {
    fn get_secret_str(&self, reference: &str) -> Option<SecretString> {
        let name = reference.replace("os.environ/", "");
        let text = self.environment.get(&name)?;
        let lowered = text.trim().to_lowercase();
        if lowered == "true" || lowered == "false" {
            return None;
        }
        Some(SecretString::from(text.clone()))
    }
}

pub(super) struct Fixture {
    pub(super) transport: Arc<FixtureTransport>,
    pub(super) clock: Arc<FixtureClock>,
    pub(super) resolver: FixtureResolver,
    pub(super) settings: Settings,
}

impl Fixture {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            transport: Arc::new(FixtureTransport::new(replies)),
            clock: Arc::new(FixtureClock::new(NOW)),
            resolver: FixtureResolver::new(&[]),
            settings: Settings::default(),
        }
    }

    pub(super) fn with_settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    pub(super) fn cache(&self) -> ProviderCache {
        let transport = Arc::clone(&self.transport);
        let clock = Arc::clone(&self.clock);
        ProviderCache::new(transport, clock, self.settings.clone())
    }

    pub(super) fn provider(&self) -> Arc<ClientSecretProvider> {
        self.provider_for(&input(), None)
    }

    pub(super) fn provider_for(
        &self,
        input: &ClientSecretInput,
        authority_host: Option<&str>,
    ) -> Arc<ClientSecretProvider> {
        let mut cache = self.cache();
        cache
            .provider(input, &self.resolver, authority_host)
            .unwrap()
    }
}

pub(super) fn form_map(request: &HttpRequest) -> BTreeMap<String, String> {
    request
        .form
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub(super) fn input() -> ClientSecretInput {
    input_for(TENANT, CLIENT, SECRET, None)
}

pub(super) fn input_for(
    tenant: &str,
    client: &str,
    secret: &str,
    scope: Option<&str>,
) -> ClientSecretInput {
    ClientSecretInput {
        tenant_id: tenant.to_owned(),
        client_id: client.to_owned(),
        client_secret: SecretString::from(secret),
        scope: scope.map(str::to_owned),
    }
}

pub(super) fn error_text(error: &ClientSecretError) -> String {
    format!("{error} {error:?}")
}
