use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_logging::rubrik_delivery::config::{RubrikEventHook, RubrikInit, RubrikSettings};
use betterlite_logging::rubrik_delivery::error::RubrikError;
use betterlite_logging::rubrik_delivery::logger::RubrikLogger;
use betterlite_logging::rubrik_delivery::post::{post_prompt_moderation, post_response_moderation};
use betterlite_logging::rubrik_delivery::queue::{
    append_and_maybe_flush, async_send_batch, flush_queue, queue_snapshot,
};
use betterlite_logging::rubrik_delivery::transport::Timeout;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

mod scripted;

use scripted::{Reply, ScriptedClock, ScriptedTransport, WarningCount};

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    environment: BTreeMap<String, String>,
    init: Init,
    replies: Vec<Reply>,
    clock: Vec<f64>,
    actions: Vec<Action>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Init {
    api_key: Option<String>,
    api_base: Option<String>,
    event_hook: Option<RubrikEventHook>,
    default_on: Option<bool>,
    guardrail_name: Option<String>,
    timeout_seconds: Option<f64>,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Action {
    Append { payload: Json },
    AppendSequence { count: u64 },
    Flush,
    SendBatch,
    PostResponse { response: Json, request: Json },
    PostPrompt { payload: Json },
}

pub(crate) struct Op;

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: fmt::Display,
{
    Box::new(HelperError {
        class: "RubrikDeliveryError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn rubrik_init(init: &Init) -> Result<RubrikInit, Box<HelperError>> {
    let timeout = match init.timeout_seconds {
        Some(seconds) => Some(
            Duration::try_from_secs_f64(seconds)
                .map_err(|error| failure(format!("init.timeout_seconds: {error}")))?,
        ),
        None => None,
    };
    Ok(RubrikInit {
        api_key: init.api_key.clone().map(SecretString::from),
        api_base: init.api_base.clone(),
        event_hook: init.event_hook,
        default_on: init.default_on,
        guardrail_name: init.guardrail_name.clone(),
        timeout,
    })
}

fn index_payload(index: u64) -> Json {
    Json::Object(IndexMap::from([(
        "index".to_owned(),
        Json::Number(serde_json::Number::from(index)),
    )]))
}

const fn classify(error: &RubrikError) -> &'static str {
    match error {
        RubrikError::HttpStatus { .. } => "http_status",
        RubrikError::NonObjectResponse { .. } => "non_object_response",
        RubrikError::ResponseBody { .. } => "invalid_body",
        RubrikError::Transport { .. } => "request_failed",
        RubrikError::Serialize { .. } => "invalid_payload",
        RubrikError::MissingWebhookUrl | RubrikError::InvalidApiKey { .. } => {
            "initialization_failed"
        }
    }
}

fn outcome(logger: &RubrikLogger, error: Option<&'static str>) -> Value {
    let queue_length = logger.state().length;
    json!({
        "queue_length": queue_length,
        "error": error,
    })
}

async fn perform(logger: &RubrikLogger, action: &Action) -> Value {
    match action {
        Action::Append { payload } => {
            append_and_maybe_flush(logger, payload.clone()).await;
            outcome(logger, None)
        }
        Action::AppendSequence { count } => {
            for index in 0..*count {
                append_and_maybe_flush(logger, index_payload(index)).await;
            }
            outcome(logger, None)
        }
        Action::Flush => {
            flush_queue(logger).await;
            outcome(logger, None)
        }
        Action::SendBatch => {
            let error = async_send_batch(logger)
                .await
                .err()
                .map(|error| classify(&error));
            outcome(logger, error)
        }
        Action::PostResponse { response, request } => {
            let error = post_response_moderation(logger, response, request)
                .await
                .err()
                .map(|error| classify(&error));
            outcome(logger, error)
        }
        Action::PostPrompt { payload } => {
            let error = post_prompt_moderation(logger, payload)
                .await
                .err()
                .map(|error| classify(&error));
            outcome(logger, error)
        }
    }
}

fn settings_value(settings: &RubrikSettings) -> Value {
    let timeout = |value: Timeout| {
        json!({
            "total": value.total.as_secs_f64(),
            "connect": value.connect.as_secs_f64(),
        })
    };
    json!({
        "guardrail_name": settings.guardrail_name,
        "event_hook": settings.event_hook.as_str(),
        "default_on": settings.default_on,
        "sampling_rate": settings.sampling_rate,
        "batch_size": settings.batch_size,
        "max_queue_size": settings.max_queue_size,
        "streaming_end_of_stream_only": settings.stream.end_of_stream_only,
        "streaming_buffer_until_moderated": settings.stream.buffer_until_moderated,
        "authenticated": settings.api_key.is_some(),
        "response_moderation_endpoint": settings.response_moderation_endpoint,
        "prompt_moderation_endpoint": settings.prompt_moderation_endpoint,
        "logging_endpoint": settings.logging_endpoint,
        "moderation_timeout": timeout(settings.moderation_timeout),
        "logging_timeout": timeout(settings.logging_timeout),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let clock = Arc::new(scripted_clock(&input.clock)?);
        let transport = Arc::new(ScriptedTransport::new(&input.replies));
        let init = rubrik_init(&input.init)?;
        let environment = input.environment.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(failure)?;
        let warnings = WarningCount::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        tracing::subscriber::with_default(subscriber, || {
            let logger = Arc::new(
                RubrikLogger::new(
                    &init,
                    &environment,
                    Arc::<ScriptedTransport>::clone(&transport),
                    clock,
                )
                .map_err(failure)?,
            );
            transport.attach(&logger);
            let results = runtime.block_on(async {
                let mut results = Vec::new();
                for action in &input.actions {
                    results.push(perform(&logger, action).await);
                }
                results
            });
            Ok(report(&logger, &transport, &warnings, &results))
        })
    }
}

fn scripted_clock(values: &[f64]) -> Result<ScriptedClock, Box<HelperError>> {
    let durations = values
        .iter()
        .map(|seconds| {
            Duration::try_from_secs_f64(*seconds)
                .map_err(|error| failure(format!("clock entry: {error}")))
        })
        .collect::<Result<Vec<Duration>, _>>()?;
    Ok(ScriptedClock::new(&durations))
}

fn report(
    logger: &RubrikLogger,
    transport: &ScriptedTransport,
    warnings: &WarningCount,
    results: &[Value],
) -> Value {
    let state = logger.state();
    let queue = queue_snapshot(logger);
    let head: Vec<Json> = queue.iter().take(3).cloned().collect();
    let tail: Vec<Json> = queue.iter().rev().take(3).rev().cloned().collect();
    json!({
        "settings": settings_value(&logger.settings),
        "results": results,
        "calls": transport.calls(),
        "queue": {"length": state.length, "head": head, "tail": tail},
        "dropped_since_warning": state.dropped_since_warning,
        "last_drop_warning_time": state.last_drop_warning_time.as_secs_f64(),
        "last_flush_time": state.last_flush_time.as_secs_f64(),
        "overflow_warning_count": warnings.0.load(Ordering::SeqCst),
    })
}
