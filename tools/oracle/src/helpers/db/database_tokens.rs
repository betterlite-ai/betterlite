use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::database_tokens::endpoint::{IAMEndpoint, parse_iam_endpoint_from_url};
use betterlite_db::database_tokens::expiry::{TokenAuthKind, parse_token_expiration};
use serde::Deserialize;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EndpointInput {
    host: String,
    port: String,
    user: String,
    name: String,
    #[serde(default)]
    schema: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    BuildUrl {
        endpoint: EndpointInput,
        token: String,
    },
    ParseEndpoint {
        url: String,
    },
    Expiration {
        kind: TokenAuthKind,
        token: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    BuildUrl {
        endpoint: EndpointInput,
        token: String,
    },
    ParseEndpoint {
        url: String,
    },
    Expiration {
        kind: TokenAuthKind,
        token: String,
    },
    Batch {
        calls: Vec<Call>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::BuildUrl { endpoint, token } => {
                Ok(serde_json::Value::String(build_url(endpoint, token)))
            }
            Input::ParseEndpoint { url } => parse_endpoint(url),
            Input::Expiration { kind, token } => expiration(*kind, token),
            Input::Batch { calls } => Ok(serde_json::Value::Array(
                calls.iter().map(scenario).collect(),
            )),
        }
    }
}

fn build_url(endpoint: &EndpointInput, token: &str) -> String {
    let parsed = IAMEndpoint {
        host: endpoint.host.clone(),
        port: endpoint.port.clone(),
        user: endpoint.user.clone(),
        name: endpoint.name.clone(),
        schema: endpoint.schema.clone(),
    };
    parsed.build_url(token)
}

fn parse_endpoint(url: &str) -> HelperResult {
    let endpoint = parse_iam_endpoint_from_url(url).map_err(|error| {
        helper_error("ValueError", format!("cannot parse IAM endpoint: {error}"))
    })?;
    Ok(serde_json::json!({
        "host": endpoint.host,
        "port": endpoint.port,
        "user": endpoint.user,
        "name": endpoint.name,
        "schema": endpoint.schema,
    }))
}

fn expiration(kind: TokenAuthKind, token: &str) -> HelperResult {
    let levels = Levels::default();
    let subscriber = tracing_subscriber::registry().with(levels.clone());
    let parsed =
        tracing::subscriber::with_default(subscriber, || parse_token_expiration(kind, token));
    let log_level = levels
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned();
    match parsed {
        Ok(value) => Ok(serde_json::json!({
            "value": value.map(|expiration| expiration.format("%Y-%m-%dT%H:%M:%S").to_string()),
            "log_level": log_level,
        })),
        Err(error) => Err(helper_error("OverflowError", error.to_string())),
    }
}

fn scenario(call: &Call) -> serde_json::Value {
    let outcome = match call {
        Call::BuildUrl { endpoint, token } => {
            Ok(serde_json::Value::String(build_url(endpoint, token)))
        }
        Call::ParseEndpoint { url } => parse_endpoint(url),
        Call::Expiration { kind, token } => expiration(*kind, token),
    };
    match outcome {
        Ok(value) => serde_json::json!({"ok": true, "value": value}),
        Err(_error) => serde_json::json!({"ok": false}),
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

#[derive(Clone, Default)]
struct Levels(Arc<Mutex<Vec<String>>>);

impl<Inner: Subscriber> Layer<Inner> for Levels {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.metadata().level().as_str().to_ascii_lowercase());
    }
}
