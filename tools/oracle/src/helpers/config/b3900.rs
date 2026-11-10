use crate::helpers::OrHelperError as _;
use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
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
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};
use logs::Logs;

mod logs;

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

trait StepEnvironment {
    fn environment(&self) -> &BTreeMap<String, String>;
}

impl StepEnvironment for EnvironmentStep {
    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
}

impl StepEnvironment for CredentialStep {
    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
}

impl StepEnvironment for HostStep {
    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
}

impl StepEnvironment for EnvironmentValueStep {
    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
}

impl StepEnvironment for FlushIntervalStep {
    fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
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
            Input::Credentials { steps } => collect(steps, &logs, credentials_value),
            Input::Host { steps } => collect(
                steps,
                &logs,
                |settings, step| json!({"ok": true, "value": resolve_host(settings, step.host.as_deref())}),
            ),
            Input::Environment { steps } => collect(steps, &logs, |settings, step| {
                verdict(
                    resolve_environment(settings, step.explicit.as_deref())
                        .map(|value| json!(value)),
                )
            }),
            Input::FlushInterval { steps } => collect(
                steps,
                &logs,
                |settings, step| json!({"ok": true, "value": flush_interval(settings, step.default)}),
            ),
            Input::Read { reading, steps } => collect(steps, &logs, |settings, _step| {
                verdict(read(*reading, settings))
            }),
        })
    }
}

fn load_settings(environment: &BTreeMap<String, String>) -> Result<Settings, Box<HelperError>> {
    Ok(ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .or_config_error()?
    .sections
    .langfuse_settings)
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

fn collect<Step, F>(steps: &[Step], logs: &Logs, value: F) -> HelperResult
where
    Step: StepEnvironment,
    F: Fn(&Settings, &Step) -> Value,
{
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let settings = load_settings(step.environment())?;
        results.push(observed(logs, value(&settings, step)));
    }
    Ok(Value::Array(results))
}

fn credentials_value(settings: &Settings, step: &CredentialStep) -> Value {
    let resolved = resolve_credentials(
        settings,
        CredentialRequest {
            public_key: step.public_key.as_deref(),
            secret: step.secret.as_deref(),
            secret_key: step.secret_key.as_deref(),
            host: step.host.as_deref(),
            allow_env_credentials: step.allow_env_credentials,
        },
    );
    json!({
        "ok": true,
        "value": {
            "public_key": resolved.public_key,
            "secret_key": resolved.secret_key.as_ref().map(secrecy::ExposeSecret::expose_secret),
            "host": resolved.host,
        },
    })
}

fn read(reading: Reading, settings: &Settings) -> Result<Value, LangfuseSettingsError> {
    match reading {
        Reading::FlushAt => Ok(json!(flush_at(settings))),
        Reading::SampleRate => Ok(json!(sample_rate(settings))),
        Reading::MaxRetries => Ok(json!(max_retries(settings))),
        Reading::Timeout => timeout_seconds(settings).map(|seconds| json!(seconds)),
        Reading::PromptCacheTtl => {
            check_prompt_cache_ttl(settings).map(|()| json!(prompt_cache_ttl_seconds(settings)))
        }
        Reading::Release => Ok(json!(release(settings))),
        Reading::DebugAndMock => debug_and_mock(settings),
    }
}

fn verdict(read: Result<Value, LangfuseSettingsError>) -> Value {
    read.map_or_else(
        |_| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    )
}

fn debug_and_mock(settings: &Settings) -> Result<Value, LangfuseSettingsError> {
    let debug = debug(settings)?;
    let mock = mock_mode(settings)?;
    warn_if_upstream_configured(settings);
    Ok(json!({"debug": debug, "mock": mock}))
}

mod tests;
