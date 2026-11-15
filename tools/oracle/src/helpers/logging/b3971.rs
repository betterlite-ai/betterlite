use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::posthog_logging::batch::QueuedEvent;
use betterlite_logging::posthog_logging::credentials::credentials_for_request;
use betterlite_logging::posthog_logging::delivery::Transport;
use betterlite_logging::posthog_logging::logger::{PostHogDeps, PostHogLogger};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio_util::task::TaskTracker;

use crate::helpers::{Helper, HelperError, HelperResult};

mod scripted;

use scripted::{AllowAllUrlCheck, ScriptedTransport};

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Init,
    EventPayload,
    Credentials,
    SyncSend,
    AsyncQueue,
    AsyncBatch,
    AsyncInit,
    ExitFlush,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsInput {
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    api_url: String,
    #[serde(default)]
    mock: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventInput {
    payload: StandardLoggingPayload,
    #[serde(default)]
    dynamic_params: Option<Json>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    #[serde(default = "default_status")]
    status: u16,
}

const fn default_status() -> u16 {
    200
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    settings: SettingsInput,
    #[serde(default)]
    metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    events: Vec<EventInput>,
    #[serde(default)]
    replies: Vec<ReplyInput>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        runtime.block_on(run_scenario(input))
    }
}
pub(super) fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "PostHogError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
fn environment(settings: &SettingsInput) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    if !settings.api_key.is_empty() {
        environment.insert("POSTHOG_API_KEY".to_owned(), settings.api_key.clone());
    }
    if !settings.api_url.is_empty() {
        environment.insert("POSTHOG_API_URL".to_owned(), settings.api_url.clone());
    }
    if settings.mock {
        environment.insert("POSTHOG_MOCK".to_owned(), "true".to_owned());
    }
    environment
}
fn logger_event(input: &Input, event: &EventInput) -> LoggerEvent {
    let mut payload = event.payload.clone();
    if let Some(metadata) = &input.metadata {
        payload.metadata.requester_metadata = Some(Json::Object(metadata.clone()));
    }
    LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(event.dynamic_params.clone().unwrap_or(Json::Null)),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}
fn async_state(logger: &PostHogLogger) -> Value {
    let initialized = logger
        .async_initialized
        .load(std::sync::atomic::Ordering::SeqCst);
    json!({"initialized": initialized, "flush_lock": initialized})
}
fn queue_value(logger: &PostHogLogger) -> Value {
    let entries: Vec<Value> = logger
        .sink
        .queue
        .pending()
        .iter()
        .map(|queued| {
            json!({
                "event": queued.event,
                "api_key": queued.api_key.expose_secret(),
                "api_url": queued.api_url,
            })
        })
        .collect();
    json!({"length": entries.len(), "entries": entries})
}
fn credentials_value(logger: &PostHogLogger, event: &LoggerEvent) -> HelperResult {
    let credentials =
        credentials_for_request(&logger.settings, &event.dynamic_params).map_err(failure)?;
    Ok(json!({
        "api_key": credentials.api_key.expose_secret(),
        "api_url": credentials.api_url,
    }))
}
fn queued_events(logger: &PostHogLogger, events: &[LoggerEvent]) -> Vec<QueuedEvent> {
    events
        .iter()
        .filter_map(|event| match logger.queued_event(event) {
            Ok(queued) => Some(queued),
            Err(error) => {
                tracing::warn!(%error, "the helper skipped an event it could not build");
                None
            }
        })
        .collect()
}

#[derive(Default)]
struct Outcome {
    built: Vec<QueuedEvent>,
    credentials: Value,
}
async fn queue_all(logger: &PostHogLogger, events: Vec<LoggerEvent>) {
    for event in events {
        let _result = logger.async_log_success_event(Arc::new(event)).await;
    }
}
async fn perform(
    operation: Operation,
    logger: &PostHogLogger,
    events: Vec<LoggerEvent>,
    tasks: &TaskTracker,
) -> Result<Outcome, Box<HelperError>> {
    let mut outcome = Outcome::default();
    match operation {
        Operation::Init => {}
        Operation::EventPayload => outcome.built = queued_events(logger, &events),
        Operation::Credentials => {
            if let Some(first) = events.first() {
                outcome.credentials = credentials_value(logger, first)?;
            }
        }
        Operation::SyncSend => {
            for event in &events {
                let _result = logger.log_success_event(event);
            }
            tasks.close();
            tasks.wait().await;
        }
        Operation::AsyncQueue => queue_all(logger, events).await,
        Operation::AsyncBatch => {
            queue_all(logger, events).await;
            logger.flush_queue().await;
        }
        Operation::AsyncInit => {
            let single: Vec<LoggerEvent> = events.into_iter().take(1).collect();
            queue_all(logger, single).await;
        }
        Operation::ExitFlush => {
            queue_all(logger, events).await;
            logger.sink.flush_on_exit().await;
        }
    }
    Ok(outcome)
}
fn report(
    logger: &PostHogLogger,
    transport: &ScriptedTransport,
    outcome: &Outcome,
    before: &Value,
) -> Value {
    json!({
        "settings": {
            "host": logger.settings.api_url,
            "capture_url": logger.settings.capture_url,
            "batch_size": logger.settings.batch.batch_size,
            "flush_interval_seconds": logger.settings.batch.flush_interval.as_secs(),
            "mock_mode": logger.settings.mock_mode,
            "max_queue_size": logger.settings.batch.max_queue_size,
        },
        "events": outcome.built.iter().map(|queued| &queued.event).collect::<Vec<_>>(),
        "credentials": outcome.credentials,
        "queue": queue_value(logger),
        "requests": transport.requests(),
        "async": async_state(logger),
        "async_before": before,
    })
}
async fn run_scenario(input: &Input) -> HelperResult {
    let settings =
        betterlite_logging::posthog_logging::settings::resolve(&environment(&input.settings))
            .map_err(failure)?;
    let transport = Arc::new(ScriptedTransport::new(&input.replies));
    let tasks = TaskTracker::new();
    let concrete = Arc::clone(&transport);
    let pooled: Arc<dyn Transport> = concrete;
    let logger = PostHogLogger::new(
        settings,
        PostHogDeps {
            transport: pooled,
            url_check: Arc::new(AllowAllUrlCheck),
            ids: || "generated-id".to_owned(),
            tasks: tasks.clone(),
            shutdown: None,
        },
    );
    let before = async_state(&logger);
    let events: Vec<LoggerEvent> = input
        .events
        .iter()
        .map(|event| logger_event(input, event))
        .collect();
    let outcome = perform(input.operation, &logger, events, &tasks).await?;
    Ok(report(&logger, &transport, &outcome, &before))
}

mod tests;
