mod client;
mod normalize;
mod provider;
mod session;

use std::sync::Arc;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::realtime_contracts::transport::RealtimeMessage;
use betterlite_core::api::transcription_protocol::events::UuidEventIds;
use betterlite_core::meta_muse_realtime::request::MetaRealtimeConfig;
use betterlite_core::meta_muse_realtime::request::Monotonic;
use betterlite_core::meta_muse_realtime::request::Sleep;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

pub(crate) type MuseConfig = MetaRealtimeConfig<UuidEventIds>;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Function {
    Normalize(normalize::NormalizeInput),
    Session(session::SessionInput),
    Request(client::RequestInput),
    Pacing(client::PacingInput),
    Events(provider::EventsInput),
    Response(Box<provider::ResponseInput>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Function;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Function::Normalize(fields) => normalize::run(fields),
            Function::Session(fields) => session::run(fields),
            Function::Request(fields) => client::request(fields),
            Function::Pacing(fields) => client::pacing(fields),
            Function::Events(fields) => provider::events(fields),
            Function::Response(fields) => provider::response(fields),
        }
    }
}

// The Muse handshake, transcript events and push-to-talk turn ids carry fresh random identifiers, so both sides fold them to a placeholder before the comparison
pub(crate) fn normalize_ids(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(normalize_ids).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), normalize_field(name, item)))
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

fn normalize_field(name: &str, value: &Value) -> Value {
    let normalized = normalize_ids(value);
    let prefix = match name {
        "event_id" => "event_",
        "item_id" => "item_",
        _ => return normalized,
    };
    let Value::String(text) = &normalized else {
        return normalized;
    };
    if !random_identifier(text, prefix) {
        return normalized;
    }
    Value::String(format!("{prefix}<random>"))
}

fn random_identifier(text: &str, prefix: &str) -> bool {
    let Some(identifier) = text.strip_prefix(prefix) else {
        return false;
    };
    let bytes = identifier.as_bytes();
    bytes.len() == 32
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

pub(crate) fn idle_config() -> MuseConfig {
    let clock: Monotonic = Arc::new(|| 0.0);
    let sleep: Sleep = Arc::new(|_delay: f64| -> BoxFuture<'static, ()> { Box::pin(async {}) });
    MetaRealtimeConfig::new(clock, sleep, ApiLimits::default(), UuidEventIds)
}

pub(crate) fn config_for(model: &str, access_token: &str) -> Result<MuseConfig, Box<HelperError>> {
    let mut config = idle_config();
    let headers = HeaderMap::new();
    drop(
        config
            .validate_environment(headers, model, Some(access_token), None)
            .or_value_error()?,
    );
    Ok(config)
}

fn message_document(message: &RealtimeMessage) -> Value {
    match message {
        RealtimeMessage::Text(text) => json!({"text": text}),
        RealtimeMessage::Binary(bytes) => json!({"binary": bytes}),
    }
}
