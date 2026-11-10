use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use betterlite_azure::azure_client_secret::ClientSecretError;
use betterlite_azure::azure_client_secret::transport::HttpMethod;
use betterlite_azure::azure_username_password::provider::{AccessToken, UsernamePasswordProvider};
use betterlite_azure::azure_username_password::record::AuthenticationRecord;
use betterlite_azure::azure_username_password::token::PasswordTokenError;
use betterlite_azure::azure_username_password::{
    DEFAULT_SCOPE, UsernamePasswordError, UsernamePasswordInput,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use super::fixture::{Capture, CapturedEvent, FixtureClock, FixtureTransport, NOW};
use super::steps::{Observe, Step};

const CREDENTIAL_ARGUMENTS: [&str; 3] = ["client_id", "password", "username"];

struct Attempt {
    transport: Arc<FixtureTransport>,
    clock: Arc<FixtureClock>,
}

impl Attempt {
    fn new(step: &Step) -> Self {
        Self {
            transport: Arc::new(FixtureTransport::new(&step.replies)),
            clock: Arc::new(FixtureClock {
                now: AtomicI64::new(NOW),
            }),
        }
    }

    fn provider(&self, step: &Step) -> Result<UsernamePasswordProvider, UsernamePasswordError> {
        let transport = Arc::clone(&self.transport);
        let clock = Arc::clone(&self.clock);
        // the Python adapter folds AZURE_AUTHORITY_HOST into the environment the SDK reads
        let configured = step.authority_host.as_deref().or_else(|| {
            step.environment
                .get("AZURE_AUTHORITY_HOST")
                .map(String::as_str)
        });
        UsernamePasswordProvider::new(&input_of(step), configured, transport, clock)
    }
}

fn input_of(step: &Step) -> UsernamePasswordInput {
    UsernamePasswordInput {
        client_id: step.client_id.clone(),
        username: step.username.clone(),
        password: SecretString::from(step.password.clone()),
        scope: step.scope.clone(),
    }
}

fn error_json(reason: &str, message: &str, token_requests: usize) -> Value {
    json!({
        "kind": "error",
        "reason": reason,
        "message": message,
        "token_requests": token_requests,
    })
}

fn token_failure(error: &UsernamePasswordError, attempt: &Attempt) -> Value {
    let token_requests = attempt.transport.posts().len();
    if attempt.transport.failed.load(Ordering::SeqCst) {
        return error_json("transport", "", token_requests);
    }
    let (reason, message) = token_reason(error, token_requests);
    error_json(reason, &message, token_requests)
}

fn token_reason(error: &UsernamePasswordError, token_requests: usize) -> (&'static str, String) {
    match error {
        UsernamePasswordError::Token(PasswordTokenError::Record(_)) => {
            ("auth_record", error.to_string())
        }
        UsernamePasswordError::ReservedScope => ("reserved_scope", error.to_string()),
        UsernamePasswordError::Token(PasswordTokenError::Shared(
            ClientSecretError::TokenExpiresInMissing,
        )) => ("missing_expires_in", String::new()),
        UsernamePasswordError::Authentication { .. } => {
            let message = if token_requests > 0 {
                error.to_string()
            } else {
                String::new()
            };
            ("authentication_failed", message)
        }
        UsernamePasswordError::Authority(_) => ("authentication_failed", String::new()),
        UsernamePasswordError::Transport { .. } => ("transport", String::new()),
        UsernamePasswordError::Token(_)
        | UsernamePasswordError::FederatedAccount
        | UsernamePasswordError::Realm(_)
        | UsernamePasswordError::ExpiryOverflow
        | UsernamePasswordError::ProviderCachePoisoned => ("other", String::new()),
    }
}

fn record_json(record: &AuthenticationRecord) -> Value {
    json!({
        "authority": &record.authority,
        "client_id": &record.client_id,
        "home_account_id": &record.home_account_id,
        "tenant_id": &record.tenant_id,
        "username": &record.username,
    })
}

fn grant_json(token: &AccessToken, step: &Step, attempt: &Attempt) -> Value {
    let requests = attempt.transport.requests();
    let Some(post) = requests
        .iter()
        .find(|request| request.method == HttpMethod::Post)
    else {
        return error_json("no_token_request", "", attempt.transport.posts().len());
    };
    let mut scopes: Vec<&str> = post
        .form
        .iter()
        .filter(|(name, _value)| name == "scope")
        .flat_map(|(_name, value)| value.split_whitespace())
        .collect();
    scopes.sort_unstable();
    let fields: BTreeMap<&str, &str> = post
        .form
        .iter()
        .filter(|(name, _value)| name != "scope")
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let discovery = requests
        .iter()
        .find(|request| request.url.contains("well-known/openid-configuration"))
        .map(|request| &request.url);
    json!({
        "kind": "token",
        "credential_arguments": CREDENTIAL_ARGUMENTS,
        "scope_arguments": [step.scope.as_deref().unwrap_or(DEFAULT_SCOPE)],
        "discovery": discovery,
        "token_url": &post.url,
        "form": fields,
        "scope": scopes,
        "token": token.token.expose_secret(),
    })
}

fn with_provider<Action>(step: &Step, action: Action) -> Value
where
    Action: FnOnce(&UsernamePasswordProvider, &Attempt) -> Value,
{
    let attempt = Attempt::new(step);
    match attempt.provider(step) {
        Ok(provider) => action(&provider, &attempt),
        Err(error) => error_json("constructor", &error.to_string(), 0),
    }
}

pub(super) fn grant_step(step: &Step) -> Value {
    with_provider(step, |provider, attempt| match provider.token() {
        Ok(token) => grant_json(&token, step, attempt),
        Err(error) => token_failure(&error, attempt),
    })
}

fn call_provider(
    provider: &UsernamePasswordProvider,
    step: &Step,
    clock: &FixtureClock,
) -> Result<Vec<AccessToken>, UsernamePasswordError> {
    let mut tokens = Vec::new();
    for index in 0..step.calls.unwrap_or(1) {
        if index > 0 {
            clock
                .now
                .store(NOW.saturating_add(step.advance), Ordering::SeqCst);
        }
        tokens.push(provider.token()?);
    }
    Ok(tokens)
}

pub(super) fn attempt_step(step: &Step) -> Value {
    with_provider(step, |provider, attempt| {
        if matches!(step.observe, Observe::Lazy) {
            return json!({
                "kind": "callable",
                "callable": true,
                "token_requests": attempt.transport.posts().len(),
            });
        }
        let acquired = match call_provider(provider, step, &attempt.clock) {
            Ok(acquired) => acquired,
            Err(error) => return token_failure(&error, attempt),
        };
        let token_requests = attempt.transport.posts().len();
        let record = acquired.last().map(|token| record_json(&token.record));
        match (step.observe, acquired.last()) {
            (Observe::Info, Some(token)) => json!({
                "kind": "info",
                "token": token.token.expose_secret(),
                "expires_on": token.expires_on,
                "token_type": &token.token_type,
                "refresh_on": token.refresh_on,
                "token_requests": token_requests,
                "record": record,
            }),
            _ => json!({
                "kind": "token",
                "tokens": acquired.iter().map(|token| token.token.expose_secret()).collect::<Vec<_>>(),
                "token_requests": token_requests,
                "record": record,
            }),
        }
    })
}

fn debug_json(events: &[CapturedEvent], step: &Step) -> Vec<Value> {
    let mut lines = Vec::new();
    for event in events.iter().filter(|event| {
        event.level == tracing::Level::DEBUG
            && (event.message.starts_with("client_id=")
                || event.message.starts_with("token_provider "))
    }) {
        let exposes = [&step.username, &step.password]
            .into_iter()
            .any(|secret| !secret.is_empty() && event.message.contains(secret.as_str()));
        if event.message.starts_with("client_id=") {
            lines.push(json!({
                "kind": "credentials",
                "client_id": event.fields.get("client_id"),
                "username_set": event.fields.get("username_set").is_some_and(|flag| flag == "true"),
                "password_set": event.fields.get("password_set").is_some_and(|flag| flag == "true"),
                "exposes_identity": exposes,
            }));
        } else {
            lines.push(json!({
                "kind": "provider",
                "identity": "present",
                "exposes_identity": exposes,
            }));
        }
    }
    lines
}

pub(super) fn construction_step(step: &Step) -> Value {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let mut value = tracing::subscriber::with_default(subscriber, || {
        with_provider(step, |provider, attempt| {
            if step.calls.is_some_and(|calls| calls > 0)
                && let Err(error) = provider.token()
            {
                return token_failure(&error, attempt);
            }
            json!({
                "kind": "constructed",
                "token_requests": attempt.transport.posts().len(),
            })
        })
    });
    let events = capture.events();
    let warnings: Vec<Value> = events
        .iter()
        .filter(|event| event.level == tracing::Level::WARN)
        .map(|event| json!({"category": event.fields.get("category"), "message": &event.message}))
        .collect();
    if let Value::Object(fields) = &mut value {
        fields.insert("warnings".to_owned(), Value::Array(warnings));
        fields.insert("debug".to_owned(), Value::Array(debug_json(&events, step)));
    }
    value
}
