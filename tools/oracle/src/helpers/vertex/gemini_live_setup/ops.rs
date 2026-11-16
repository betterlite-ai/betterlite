use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::studio_identity;
use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_models::catalog::BundledCatalog;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::gemini_live_setup::automatic::session_configuration_request;
use betterlite_vertex::gemini_live_setup::created::session_created;
use betterlite_vertex::gemini_live_setup::endpoint::connect_url;
use betterlite_vertex::gemini_live_setup::session::SessionUpdate;
use betterlite_vertex::gemini_live_setup::settings::LiveSetupSettings;
use betterlite_vertex::gemini_live_setup::setup::transform_session_update;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use super::warnings::Warnings;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

const DEFAULT_EVENT_ID: &str = "00000000-0000-4000-8000-000000000000";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    SessionUpdate,
    Automatic,
    Created,
    Endpoint,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    op: Operation,
    model: String,
    #[serde(default)]
    session: Object,
    #[serde(default)]
    session_configuration_request: Option<String>,
    #[serde(default)]
    defer_setup: bool,
    #[serde(default)]
    logging_session_id: String,
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let schema = SchemaSettings::default();
    let mut calls = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        let settings = LiveSetupSettings {
            catalog: catalog.as_ref(),
            schema: &schema,
            defer_setup: call.defer_setup,
        };
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        let value = tracing::subscriber::with_default(subscriber, || dispatch(call, &settings))?;
        calls.push(json!({"value": value, "warnings": warnings.labels()}));
    }
    Ok(json!({"calls": calls}))
}

fn dispatch(call: &Call, settings: &LiveSetupSettings<'_>) -> Result<Value, Box<HelperError>> {
    match call.op {
        Operation::SessionUpdate => session_update(call, settings),
        Operation::Automatic => automatic(call, settings),
        Operation::Created => created(call),
        Operation::Endpoint => endpoint(call),
    }
}

fn session_update(
    call: &Call,
    settings: &LiveSetupSettings<'_>,
) -> Result<Value, Box<HelperError>> {
    let session_value = serde_json::to_value(&call.session).or_value_error()?;
    let update = serde_json::from_value::<SessionUpdate>(session_value).or_value_error()?;
    let messages = transform_session_update(
        &update,
        &call.model,
        call.session_configuration_request.as_deref(),
        settings,
    )
    .or_value_error()?;
    let parsed = messages
        .iter()
        .map(|message| serde_json::from_str::<Value>(message).or_value_error())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"messages": parsed}))
}

fn automatic(call: &Call, settings: &LiveSetupSettings<'_>) -> Result<Value, Box<HelperError>> {
    let request = session_configuration_request(&call.model, settings).or_value_error()?;
    let setup = match request.as_deref() {
        Some(text) => serde_json::from_str::<Value>(text)
            .or_value_error()?
            .get("setup")
            .cloned()
            .unwrap_or(Value::Null),
        None => Value::Null,
    };
    Ok(json!({"required": request.is_some(), "setup": setup}))
}

fn created(call: &Call) -> Result<Value, Box<HelperError>> {
    let event = session_created(
        &call.model,
        &call.logging_session_id,
        call.session_configuration_request.as_deref(),
        call.event_id.as_deref().unwrap_or(DEFAULT_EVENT_ID),
    )
    .or_value_error()?;
    serde_json::to_value(event).or_value_error()
}

fn endpoint(call: &Call) -> Result<Value, Box<HelperError>> {
    let identity = studio_identity::Raw::default()
        .convert(&call.env)
        .or_value_error()?;
    let explicit = call
        .api_key
        .as_deref()
        .map(|key| SecretString::from(key.to_owned()));
    let url = connect_url(
        call.api_base.as_deref(),
        explicit.as_ref(),
        identity.environment_key.as_ref(),
    )
    .or_value_error()?;
    Ok(json!({"url": url.expose_secret()}))
}
