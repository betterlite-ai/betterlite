use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::SecretString;

use crate::azure_client_secret::transport::{Clock, HttpMethod, HttpReply};
use crate::azure_username_password::provider::UsernamePasswordProvider;
use crate::azure_username_password::transport::{HttpRequest, Transport};
use crate::azure_username_password::{UsernamePasswordError, UsernamePasswordInput};

pub(super) const HOST: &str = "login.microsoftonline.com";
pub(super) const TENANT: &str = "organizations";
pub(super) const CLIENT: &str = "client";
pub(super) const USERNAME: &str = "user@example.com";
pub(super) const PASSWORD: &str = "fixture-password";
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

pub(super) fn openid(host: &str) -> Reply {
    Reply::ok(
        "well-known/openid-configuration",
        &format!(
            r#"{{"issuer":"https://{host}/{TENANT}/v2.0","authorization_endpoint":"https://{host}/{TENANT}/oauth2/v2.0/authorize","token_endpoint":"https://{host}/{TENANT}/oauth2/v2.0/token"}}"#
        ),
    )
}

pub(super) fn token(body: &str) -> Reply {
    Reply::ok("/oauth2/v2.0/token", body)
}

pub(super) fn instance_metadata(tenant_discovery: &str) -> Reply {
    Reply::ok(
        "/common/discovery/instance",
        &format!(r#"{{"tenant_discovery_endpoint":"{tenant_discovery}"}}"#),
    )
}

pub(super) struct FixtureTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FixtureTransport {
    pub(super) fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
            let position = replies.iter().position(|reply| {
                reply
                    .url_contains
                    .as_ref()
                    .is_none_or(|needle| request.url.contains(needle))
            });
            // a managed account is the common case, so a test that is not about the realm needs no reply for it
            match position {
                Some(index) => replies
                    .remove(index)
                    .ok_or_else(|| io::Error::other("no reply"))?,
                None if request.url.contains("/common/userrealm/") => Reply::ok("", "{}"),
                None => return Err(io::Error::other(format!("no reply for {}", request.url))),
            }
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
    pub(super) fn set(&self, now: i64) {
        self.now.store(now, Ordering::SeqCst);
    }
}

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

pub(super) struct Fixture {
    pub(super) transport: Arc<FixtureTransport>,
    pub(super) clock: Arc<FixtureClock>,
}

impl Fixture {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            transport: Arc::new(FixtureTransport {
                replies: Mutex::new(VecDeque::from(replies)),
                requests: Mutex::new(Vec::new()),
            }),
            clock: Arc::new(FixtureClock {
                now: AtomicI64::new(NOW),
            }),
        }
    }

    pub(super) fn provider(
        &self,
        input: &UsernamePasswordInput,
    ) -> Result<UsernamePasswordProvider, UsernamePasswordError> {
        self.provider_on(input, None)
    }

    pub(super) fn provider_on(
        &self,
        input: &UsernamePasswordInput,
        authority_host: Option<&str>,
    ) -> Result<UsernamePasswordProvider, UsernamePasswordError> {
        let transport = Arc::clone(&self.transport);
        let clock = Arc::clone(&self.clock);
        UsernamePasswordProvider::new(input, authority_host, transport, clock)
    }
}

pub(super) fn input(
    client_id: &str,
    username: &str,
    password: &str,
    scope: Option<&str>,
) -> UsernamePasswordInput {
    UsernamePasswordInput {
        client_id: client_id.to_owned(),
        username: username.to_owned(),
        password: SecretString::from(password.to_owned()),
        scope: scope.map(str::to_owned),
    }
}

pub(super) fn input_for(
    username: &str,
    password: &str,
    scope: Option<&str>,
) -> UsernamePasswordInput {
    input(CLIENT, username, password, scope)
}

pub(super) fn form_map(request: &HttpRequest) -> BTreeMap<String, String> {
    request.form.iter().cloned().collect()
}

pub(super) fn scope_set(request: &HttpRequest) -> Vec<String> {
    let form = form_map(request);
    let mut scopes: Vec<String> = form
        .get("scope")
        .map(|scope| scope.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    scopes.sort_unstable();
    scopes
}

pub(super) fn well_known_url(host: &str) -> String {
    format!("https://{host}/{TENANT}/v2.0/.well-known/openid-configuration")
}

pub(super) fn token_url(host: &str) -> String {
    format!("https://{host}/{TENANT}/oauth2/v2.0/token")
}
