use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_client_secret::SecretResolver;
use betterlite_azure::oidc_exchange::transport::{Clock, HttpReply, Transport};
use betterlite_azure::oidc_exchange::{AzureOidcExchange, OidcExchangeInput};
use betterlite_config::loader::ConfigLoader;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

const FIXTURE_NOW: f64 = 1_700_000_000.0;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    calls: Vec<Call>,
    #[serde(default)]
    replies: Vec<Reply>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    azure_ad_token: SecretString,
    #[serde(default)]
    azure_client_id: Option<String>,
    #[serde(default)]
    azure_tenant_id: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    advance: f64,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Outcome {
    Token { token: String },
    Error { status: Option<u16> },
}

#[derive(serde::Serialize)]
struct RequestReport {
    url: String,
    form: BTreeMap<String, String>,
}

struct Recorded {
    url: String,
    form: Vec<(String, SecretString)>,
}

struct FixtureTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Recorded>>,
}

impl FixtureTransport {
    fn report(&self) -> Vec<RequestReport> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|request| RequestReport {
                url: request.url.clone(),
                form: request
                    .form
                    .iter()
                    .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                    .collect(),
            })
            .collect()
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

struct FixtureClock {
    now: Mutex<f64>,
}

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> f64 {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct FixtureResolver {
    environment: BTreeMap<String, String>,
}

impl SecretResolver for FixtureResolver {
    fn get_secret_str(&self, reference: &str) -> Option<SecretString> {
        let name = reference.strip_prefix("os.environ/")?;
        self.environment
            .get(name)
            .map(|value| SecretString::from(value.as_str()))
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn call_outcome(exchange: &AzureOidcExchange, resolver: &FixtureResolver, call: &Call) -> Outcome {
    let input = OidcExchangeInput {
        azure_ad_token: call.azure_ad_token.clone(),
        azure_client_id: call.azure_client_id.clone(),
        azure_tenant_id: call.azure_tenant_id.clone(),
        scope: call.scope.clone(),
    };
    match exchange.token(&input, resolver) {
        Ok(token) => Outcome::Token {
            token: token.expose_secret().to_owned(),
        },
        Err(error) => Outcome::Error {
            status: error.status_code(),
        },
    }
}

fn run_step(step: &Step) -> HelperResult {
    let config = ConfigLoader {
        lookup: step.environment.clone(),
    }
    .load("")
    .map_err(|error| failure("ConfigError", error.to_string()))?;
    let transport = Arc::new(FixtureTransport {
        replies: Mutex::new(VecDeque::from(step.replies.clone())),
        requests: Mutex::new(Vec::new()),
    });
    let clock = Arc::new(FixtureClock {
        now: Mutex::new(FIXTURE_NOW),
    });
    let pool = Arc::clone(&transport);
    let ticking = Arc::clone(&clock);
    let exchange = AzureOidcExchange::new(
        config.sections.azure_oidc_exchange,
        step.environment.clone(),
        pool,
        ticking,
    )
    .map_err(|error| failure("OidcExchangeError", error.to_string()))?;
    let resolver = FixtureResolver {
        environment: step.environment.clone(),
    };
    let mut outcomes = Vec::with_capacity(step.calls.len());
    for call in &step.calls {
        let mut now = clock.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.mul_add(1.0, call.advance);
        drop(now);
        outcomes.push(call_outcome(&exchange, &resolver, call));
    }
    Ok(json!({
        "outcomes": outcomes,
        "requests": transport.report(),
    }))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut reports: Vec<Value> = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            reports.push(run_step(step)?);
        }
        Ok(Value::Array(reports))
    }
}
