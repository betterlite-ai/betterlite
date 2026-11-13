use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::argilla_logging::extraction::{
    self, ArgillaItem, CallbackKwargs, PayloadError, StandardLoggingPayload,
};
use betterlite_logging::argilla_logging::hooks::DatasetHook;
use betterlite_logging::argilla_logging::logger::ArgillaLogger;
use betterlite_logging::argilla_logging::settings::{ArgillaSettings, ConstructorArguments};
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use self::scripted::{HookEffect, ScriptedHook, ScriptedRandom, ScriptedTransport};

mod scripted;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    arguments: Arguments,
    #[serde(default)]
    transformation_object: Option<Json>,
    #[serde(default)]
    batch_size: Option<usize>,
    #[serde(default)]
    random: Vec<f64>,
    #[serde(default)]
    replies: Vec<scripted::Reply>,
    #[serde(default)]
    hooks: Vec<HookEffect>,
    #[serde(default)]
    actions: Vec<Action>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Arguments {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    dataset_name: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    SyncSuccess {
        #[serde(default)]
        kwargs: CallbackKwargs,
    },
    AsyncSuccess {
        #[serde(default)]
        kwargs: CallbackKwargs,
    },
    AsyncFailure {
        #[serde(default)]
        kwargs: CallbackKwargs,
    },
    Flush,
    ChatMessages {
        payload: StandardLoggingPayload,
    },
    Response {
        payload: StandardLoggingPayload,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
        let mut results = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            results.push(runtime.block_on(run_scenario(scenario)));
        }
        Ok(json!({ "results": results }))
    }
}

async fn run_scenario(scenario: &Scenario) -> Value {
    let transport = ScriptedTransport {
        replies: Arc::new(Mutex::new(scenario.replies.iter().cloned().collect())),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let Some(mut logger) = connect(scenario, &transport).await else {
        let calls = transport
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        return json!({ "initialized": false, "calls": calls });
    };
    if let Some(size) = scenario.batch_size {
        logger.queue.settings.batch_size = size;
    }
    let hooks: Vec<ScriptedHook> = scenario
        .hooks
        .iter()
        .map(|effect| ScriptedHook {
            effect: *effect,
            seen: Mutex::new(Vec::new()),
            payload_present: AtomicBool::new(false),
        })
        .collect();
    let mut action_results = Vec::with_capacity(scenario.actions.len());
    for action in &scenario.actions {
        action_results.push(perform(action, &logger, &hooks).await);
    }
    report(&logger, &transport, &action_results, &hooks)
}

#[derive(serde::Serialize)]
struct Report<'report> {
    initialized: bool,
    credentials: CredentialsReport<'report>,
    sampling_rate: f64,
    batch_size: usize,
    flush_interval: u64,
    actions: &'report [Value],
    records: Vec<ArgillaItem>,
    calls: Vec<Value>,
    hooks: Vec<Value>,
}

#[derive(serde::Serialize)]
struct CredentialsReport<'report> {
    dataset_name: &'report str,
    base_url: &'report str,
    api_key: &'report str,
}

fn report(
    logger: &ArgillaLogger<ScriptedTransport, ScriptedRandom>,
    transport: &ScriptedTransport,
    actions: &[Value],
    hooks: &[ScriptedHook],
) -> Value {
    let calls = transport
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let report = Report {
        initialized: true,
        credentials: CredentialsReport {
            dataset_name: &logger.credentials.dataset_name,
            base_url: &logger.credentials.base_url,
            api_key: logger.credentials.api_key.expose_secret(),
        },
        sampling_rate: logger.sampling_rate,
        batch_size: logger.queue.settings.batch_size,
        flush_interval: logger.queue.settings.flush_interval.as_secs(),
        actions,
        records: logger.queue.pending(),
        calls,
        hooks: hook_report(hooks),
    };
    serde_json::to_value(report).unwrap_or_default()
}

async fn connect(
    scenario: &Scenario,
    transport: &ScriptedTransport,
) -> Option<ArgillaLogger<ScriptedTransport, ScriptedRandom>> {
    let arguments = ConstructorArguments {
        api_key: scenario.arguments.api_key.clone().map(SecretString::from),
        dataset_name: scenario.arguments.dataset_name.clone(),
        base_url: scenario.arguments.base_url.clone(),
    };
    let settings = ArgillaSettings::from_environment(
        &scenario.environment,
        &arguments,
        scenario.transformation_object.as_ref(),
    )
    .ok()?;
    let random = ScriptedRandom {
        values: Mutex::new(scenario.random.iter().copied().collect()),
    };
    ArgillaLogger::connect(settings, transport.clone(), random)
        .await
        .ok()
}

fn hook_report(hooks: &[ScriptedHook]) -> Vec<Value> {
    hooks
        .iter()
        .map(|hook| {
            let seen = hook
                .seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            json!({
                "effect": hook.effect,
                "seen": seen,
                "payload_present": hook.payload_present.load(Ordering::SeqCst),
            })
        })
        .collect()
}

async fn perform(
    action: &Action,
    logger: &ArgillaLogger<ScriptedTransport, ScriptedRandom>,
    hooks: &[ScriptedHook],
) -> Value {
    match action {
        Action::SyncSuccess { kwargs } => {
            logger.log_success_event(kwargs).await;
            Value::Null
        }
        Action::AsyncSuccess { kwargs } => {
            let mut borrowed: Vec<&dyn DatasetHook> = Vec::with_capacity(hooks.len());
            for hook in hooks {
                borrowed.push(hook);
            }
            logger.async_log_success_event(kwargs, &borrowed).await;
            Value::Null
        }
        Action::AsyncFailure { kwargs } => {
            logger.async_log_failure_event(kwargs).await;
            Value::Null
        }
        Action::Flush => {
            let report = logger.flush_queue().await;
            json!({ "batch": report.batch })
        }
        Action::ChatMessages { payload } => match extraction::chat_messages(payload) {
            Ok(messages) => json!({ "messages": messages }),
            Err(error) => json!({ "raised": classify(&error) }),
        },
        Action::Response { payload } => match extraction::response_value(payload) {
            Ok(response) => json!({ "response": response }),
            Err(error) => json!({ "raised": classify(&error) }),
        },
    }
}

const fn classify(error: &PayloadError) -> &'static str {
    match error {
        PayloadError::MissingChatMessages => "missing_chat_messages",
        PayloadError::InvalidChatMessages { .. } => "invalid_chat_messages",
        PayloadError::MissingResponse => "missing_response",
        PayloadError::InvalidResponse { .. } => "invalid_response",
        PayloadError::ResponseShape => "response_shape",
        PayloadError::MissingPayload => "missing_payload",
    }
}

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    let error = HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}
