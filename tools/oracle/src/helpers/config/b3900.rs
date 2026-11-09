use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::langfuse_settings::Settings;
use betterlite_logging::langfuse_settings::export::{
    check_prompt_cache_ttl, flush_at, flush_interval, max_retries, prompt_cache_ttl_seconds,
    sample_rate, timeout_seconds,
};
use betterlite_logging::langfuse_settings::{
    CredentialRequest, LangfuseSettingsError, debug, mock_mode, release, resolve_credentials,
    resolve_environment, resolve_host, warn_if_upstream_configured,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentStep {
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialStep {
    #[serde(default)]
    public_key: Option<String>,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    secret_key: Option<String>,
    #[serde(default)]
    host: Option<String>,
    allow_env_credentials: bool,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostStep {
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentValueStep {
    #[serde(default)]
    explicit: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlushIntervalStep {
    default: u64,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reading {
    FlushAt,
    SampleRate,
    MaxRetries,
    Timeout,
    PromptCacheTtl,
    Release,
    DebugAndMock,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Credentials {
        steps: Vec<CredentialStep>,
    },
    Host {
        steps: Vec<HostStep>,
    },
    Environment {
        steps: Vec<EnvironmentValueStep>,
    },
    FlushInterval {
        steps: Vec<FlushIntervalStep>,
    },
    Read {
        reading: Reading,
        steps: Vec<EnvironmentStep>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let logs = Logs::default();
        let subscriber = tracing_subscriber::registry().with(logs.clone());
        with_default(subscriber, || match input {
            Input::Credentials { steps } => credentials(steps, &logs),
            Input::Host { steps } => hosts(steps, &logs),
            Input::Environment { steps } => environments(steps, &logs),
            Input::FlushInterval { steps } => intervals(steps, &logs),
            Input::Read { reading, steps } => readings(*reading, steps, &logs),
        })
    }
}

fn load_settings(environment: &BTreeMap<String, String>) -> Result<Settings, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .map(|config| config.sections.langfuse_settings)
    .map_err(|error| config_failure(&error))
}

fn config_failure(error: &ConfigError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ConfigError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn observed(logs: &Logs, value: Value) -> Value {
    let (warnings, details, infos) = logs.take();
    let Value::Object(mut fields) = value else {
        return value;
    };
    fields.insert("warnings".to_owned(), json!(warnings));
    fields.insert("warning_details".to_owned(), json!(details));
    fields.insert("infos".to_owned(), json!(infos));
    Value::Object(fields)
}

fn credentials(steps: &[CredentialStep], logs: &Logs) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let settings = load_settings(&step.environment)?;
        let resolved = resolve_credentials(
            &settings,
            CredentialRequest {
                public_key: step.public_key.as_deref(),
                secret: step.secret.as_deref(),
                secret_key: step.secret_key.as_deref(),
                host: step.host.as_deref(),
                allow_env_credentials: step.allow_env_credentials,
            },
        );
        results.push(observed(
            logs,
            json!({
                "ok": true,
                "value": {
                    "public_key": resolved.public_key,
                    "secret_key": resolved.secret_key.as_ref().map(secrecy::ExposeSecret::expose_secret),
                    "host": resolved.host,
                },
            }),
        ));
    }
    Ok(Value::Array(results))
}

fn hosts(steps: &[HostStep], logs: &Logs) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let settings = load_settings(&step.environment)?;
        results.push(observed(
            logs,
            json!({"ok": true, "value": resolve_host(&settings, step.host.as_deref())}),
        ));
    }
    Ok(Value::Array(results))
}

fn environments(steps: &[EnvironmentValueStep], logs: &Logs) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let settings = load_settings(&step.environment)?;
        let value = resolve_environment(&settings, step.explicit.as_deref()).map_or_else(
            |_| json!({"ok": false}),
            |value| json!({"ok": true, "value": value}),
        );
        results.push(observed(logs, value));
    }
    Ok(Value::Array(results))
}

fn intervals(steps: &[FlushIntervalStep], logs: &Logs) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let settings = load_settings(&step.environment)?;
        let value = flush_interval(&settings, step.default);
        results.push(observed(logs, json!({"ok": true, "value": value})));
    }
    Ok(Value::Array(results))
}

fn readings(reading: Reading, steps: &[EnvironmentStep], logs: &Logs) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let settings = load_settings(&step.environment)?;
        let value = read(reading, &settings).map_or_else(
            |_| json!({"ok": false}),
            |value| json!({"ok": true, "value": value}),
        );
        results.push(observed(logs, value));
    }
    Ok(Value::Array(results))
}

fn read(reading: Reading, settings: &Settings) -> Result<Value, LangfuseSettingsError> {
    match reading {
        Reading::FlushAt => Ok(json!(flush_at(settings))),
        Reading::SampleRate => Ok(json!(sample_rate(settings))),
        Reading::MaxRetries => Ok(json!(max_retries(settings))),
        Reading::Timeout => Ok(json!(timeout_seconds(settings)?)),
        Reading::PromptCacheTtl => {
            check_prompt_cache_ttl(settings)?;
            Ok(json!(prompt_cache_ttl_seconds(settings)))
        }
        Reading::Release => Ok(json!(release(settings))),
        Reading::DebugAndMock => {
            let debug = debug(settings)?;
            let mock = mock_mode(settings)?;
            warn_if_upstream_configured(settings);
            Ok(json!({"debug": debug, "mock": mock}))
        }
    }
}

#[derive(Clone, Default)]
struct Logs {
    warnings: Arc<Mutex<Vec<String>>>,
    infos: Arc<Mutex<Vec<String>>>,
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::INFO {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        let target = if level == Level::WARN {
            &self.warnings
        } else {
            &self.infos
        };
        target
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

impl Logs {
    fn take(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let warnings = {
            let mut guard = self.warnings.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };
        let infos = {
            let mut guard = self.infos.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *guard)
        };
        let mut fixed = Vec::with_capacity(warnings.len());
        let mut details = Vec::with_capacity(warnings.len());
        for warning in warnings {
            let (message, detail) = warning.split_once(": ").unwrap_or((&warning, ""));
            fixed.push(message.to_owned());
            details.push(detail.to_owned());
        }
        (fixed, details, infos)
    }
}

mod tests;
