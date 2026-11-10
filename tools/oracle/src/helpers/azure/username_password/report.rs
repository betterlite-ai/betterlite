use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use betterlite_azure::azure_client_secret::ClientSecretError;
use betterlite_azure::azure_client_secret::transport::HttpMethod;
use betterlite_azure::azure_username_password::provider::{AccessToken, UsernamePasswordProvider};
use betterlite_azure::azure_username_password::record::AuthenticationRecord;
use betterlite_azure::azure_username_password::token::PasswordTokenError;
use betterlite_azure::azure_username_password::transport::HttpRequest;
use betterlite_azure::azure_username_password::{
    DEFAULT_SCOPE, UsernamePasswordError, UsernamePasswordInput,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use super::fixture::{Capture, CapturedEvent, FieldValue, FixtureClock, FixtureTransport, NOW};
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
        UsernamePasswordError::Token(PasswordTokenError::Record { .. }) => {
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
        | UsernamePasswordError::ExpiryOverflow
        | UsernamePasswordError::ProviderCachePoisoned => ("other", String::new()),
    }
}

fn resolved_scope(step: &Step) -> String {
    step.scope
        .clone()
        .unwrap_or_else(|| DEFAULT_SCOPE.to_owned())
}

fn first_post_form(requests: &[HttpRequest]) -> Option<BTreeMap<String, String>> {
    requests
        .iter()
        .find(|request| request.method == HttpMethod::Post)
        .map(|request| request.form.iter().cloned().collect())
}

fn first_post_url(requests: &[HttpRequest]) -> Option<String> {
    requests
        .iter()
        .find(|request| request.method == HttpMethod::Post)
        .map(|request| request.url.clone())
}

fn discovery_url(requests: &[HttpRequest]) -> Option<String> {
    requests
        .iter()
        .find(|request| request.url.contains("well-known/openid-configuration"))
        .map(|request| request.url.clone())
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
    let Some(form) = first_post_form(&requests) else {
        return error_json("no_token_request", "", attempt.transport.posts().len());
    };
    let scope = form.get("scope").cloned().unwrap_or_default();
    let mut scopes: Vec<&str> = scope.split_whitespace().collect();
    scopes.sort_unstable();
    let fields: BTreeMap<&str, &str> = form
        .iter()
        .filter(|(name, _value)| name.as_str() != "scope")
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    json!({
        "kind": "token",
        "credential_arguments": CREDENTIAL_ARGUMENTS,
        "scope_arguments": [resolved_scope(step)],
        "discovery": discovery_url(&requests),
        "token_url": first_post_url(&requests),
        "form": fields,
        "scope": scopes,
        "token": token.token.expose_secret(),
    })
}

pub(super) fn grant_step(step: &Step) -> Value {
    let attempt = Attempt::new(step);
    let provider = match attempt.provider(step) {
        Ok(provider) => provider,
        Err(error) => return error_json("constructor", &error.to_string(), 0),
    };
    match provider.token() {
        Ok(token) => grant_json(&token, step, &attempt),
        Err(error) => token_failure(&error, &attempt),
    }
}

fn call_provider(
    provider: &UsernamePasswordProvider,
    step: &Step,
    clock: &FixtureClock,
) -> Result<Vec<AccessToken>, UsernamePasswordError> {
    let mut tokens = Vec::new();
    for index in 0..step.calls {
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
    let attempt = Attempt::new(step);
    let provider = match attempt.provider(step) {
        Ok(provider) => provider,
        Err(error) => return error_json("constructor", &error.to_string(), 0),
    };
    match call_provider(&provider, step, &attempt.clock) {
        Ok(acquired) => {
            let record = acquired
                .last()
                .map_or(Value::Null, |token| record_json(&token.record));
            let tokens: Vec<&str> = acquired
                .iter()
                .map(|token| token.token.expose_secret())
                .collect();
            json!({
                "kind": "token",
                "tokens": tokens,
                "token_requests": attempt.transport.posts().len(),
                "record": record,
            })
        }
        Err(error) => token_failure(&error, &attempt),
    }
}

pub(super) fn info_step(step: &Step) -> Value {
    let attempt = Attempt::new(step);
    let provider = match attempt.provider(step) {
        Ok(provider) => provider,
        Err(error) => return error_json("constructor", &error.to_string(), 0),
    };
    match provider.token() {
        Ok(token) => json!({
            "kind": "info",
            "token": token.token.expose_secret(),
            "expires_on": token.expires_on,
            "token_type": &token.token_type,
            "refresh_on": token.refresh_on,
            "token_requests": attempt.transport.posts().len(),
            "record": record_json(&token.record),
        }),
        Err(error) => token_failure(&error, &attempt),
    }
}

pub(super) fn lazy_step(step: &Step) -> Value {
    let attempt = Attempt::new(step);
    match attempt.provider(step) {
        Ok(_provider) => json!({
            "kind": "callable",
            "callable": true,
            "token_requests": 0,
        }),
        Err(error) => error_json("constructor", &error.to_string(), 0),
    }
}

pub(super) fn token_response_step(step: &Step) -> Value {
    match step.observe {
        Observe::Lazy => lazy_step(step),
        Observe::Info => info_step(step),
        Observe::Token => attempt_step(step),
    }
}

fn warnings_json(events: &[CapturedEvent]) -> Value {
    let warnings: Vec<Value> = events
        .iter()
        .filter(|event| event.level == tracing::Level::WARN)
        .map(|event| {
            let category = match event.fields.get("category") {
                Some(FieldValue::Text(text)) => text.clone(),
                Some(FieldValue::Bool(_)) | None => String::new(),
            };
            json!({"category": category, "message": &event.message})
        })
        .collect();
    Value::Array(warnings)
}

fn exposes_identity(message: &str, step: &Step) -> bool {
    (!step.username.is_empty() && message.contains(&step.username))
        || (!step.password.is_empty() && message.contains(&step.password))
}

fn debug_json(events: &[CapturedEvent], step: &Step) -> Value {
    let mut lines = Vec::new();
    for event in events
        .iter()
        .filter(|event| event.level == tracing::Level::DEBUG)
    {
        if !event.message.starts_with("client_id=") && !event.message.starts_with("token_provider ")
        {
            continue;
        }
        if event.message.starts_with("client_id=") {
            let client_id = match event.fields.get("client_id") {
                Some(FieldValue::Text(text)) => text.clone(),
                Some(FieldValue::Bool(_)) | None => String::new(),
            };
            let username_set = matches!(
                event.fields.get("username_set"),
                Some(FieldValue::Bool(true))
            );
            let password_set = matches!(
                event.fields.get("password_set"),
                Some(FieldValue::Bool(true))
            );
            lines.push(json!({
                "kind": "credentials",
                "client_id": client_id,
                "username_set": username_set,
                "password_set": password_set,
                "exposes_identity": exposes_identity(&event.message, step),
            }));
        } else {
            lines.push(json!({
                "kind": "provider",
                "identity": "present",
                "exposes_identity": exposes_identity(&event.message, step),
            }));
        }
    }
    Value::Array(lines)
}

pub(super) fn construction_step(step: &Step) -> Value {
    let attempt = Attempt::new(step);
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let outcome = tracing::subscriber::with_default(subscriber, || {
        let provider = match attempt.provider(step) {
            Ok(provider) => provider,
            Err(error) => return Err((false, error)),
        };
        if step.calls > 0 {
            provider.token().map_err(|error| (true, error))?;
        }
        Ok(())
    });
    let events = capture.events();
    let warnings = warnings_json(&events);
    let debug = debug_json(&events, step);
    match outcome {
        Ok(()) => json!({
            "kind": "constructed",
            "warnings": warnings,
            "debug": debug,
            "token_requests": attempt.transport.posts().len(),
        }),
        Err((invoked, error)) => {
            let mut value = if invoked {
                token_failure(&error, &attempt)
            } else {
                error_json("constructor", &error.to_string(), 0)
            };
            if let Value::Object(fields) = &mut value {
                fields.insert("warnings".to_owned(), warnings);
                fields.insert("debug".to_owned(), debug);
            }
            value
        }
    }
}
