use std::sync::Arc;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::gcs_logger::gcs_bucket_logging::async_send_behaviors::async_send_batch;
use betterlite_logging::gcs_logger::gcs_bucket_logging::config_key_behaviors::{
    config_key, sanitize_config_key,
};
use betterlite_logging::gcs_logger::gcs_bucket_logging::drain_batch_behaviors::drain_queue_batch;
use betterlite_logging::gcs_logger::gcs_bucket_logging::event_enqueue_behaviors::enqueue;
use betterlite_logging::gcs_logger::gcs_bucket_logging::flush_behaviors::flush_queue;
use betterlite_logging::gcs_logger::gcs_bucket_logging::grouped_batch_behaviors::send_grouped_batch;
use betterlite_logging::gcs_logger::gcs_bucket_logging::grouping_behaviors::group_items_by_config;
use betterlite_logging::gcs_logger::gcs_bucket_logging::naming_format_behaviors::get_object_date;
use betterlite_logging::gcs_logger::gcs_bucket_logging::ndjson_behaviors::combine_payloads_to_ndjson;
use betterlite_logging::gcs_logger::gcs_bucket_logging::payload_retrieval_behaviors::get_request_response_payload;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::Session;
use super::failure;
use super::input::{Action, Input, Step};
use super::recording::uploads;
use crate::helpers::{HelperError, OrHelperError as _};

pub(super) async fn run_step(
    step: &Step,
    input: &Input,
    session: &Session,
) -> Result<Value, Box<HelperError>> {
    match step.action {
        Action::Event | Action::Failure => event_step(step, input, session).await,
        Action::Settings => Ok(settings_step(session)),
        Action::Drain => Ok(drain_step(session)),
        Action::Config => config_step(input, session),
        Action::Group => group_step(session),
        Action::Ndjson => ndjson_step(session),
        Action::Grouped => Ok(grouped_step(session).await),
        Action::Send => Ok(send_step(session).await),
        Action::Retrieve => retrieve_step(input, session).await,
        Action::Date => date_step(input),
        Action::Flush => Ok(flush_step(session).await),
        Action::Wait => Ok(wait_step(session).await),
    }
}

async fn event_step(
    step: &Step,
    input: &Input,
    session: &Session,
) -> Result<Value, Box<HelperError>> {
    let Some(base) = &input.payload else {
        return Err(failure("ValueError", "an event step needs a payload"));
    };
    let Some(id) = &step.id else {
        return Err(failure("ValueError", "an event step needs an id"));
    };
    let event = Arc::new(LoggerEvent {
        payload: Arc::new(payload_for(base, id, step.response_id.as_deref())),
        dynamic_params: Arc::new(step.dynamic.clone().unwrap_or(Json::Null)),
        litellm_metadata: Arc::new(step.metadata.clone().unwrap_or_default()),
        stream: false,
        guardrail_snapshot: None,
    });
    enqueue(&session.logger, &event).await;
    Ok(Value::Null)
}

// the response object LiteLLM names a success object after reaches the logger as response.id
fn payload_for(
    base: &StandardLoggingPayload,
    id: &str,
    response_id: Option<&str>,
) -> StandardLoggingPayload {
    let mut payload = base.clone();
    payload.id = id.to_owned();
    payload.response = response_id.map(|id| {
        Json::Object(IndexMap::from([(
            "id".to_owned(),
            Json::Text(id.to_owned()),
        )]))
    });
    payload
}

fn settings_step(session: &Session) -> Value {
    let settings = session.logger.settings;
    json!([
        settings.batch_size,
        settings.flush_interval.as_secs(),
        settings.use_batched_logging,
        settings.queue_capacity,
    ])
}

fn drain_step(session: &Session) -> Value {
    let items = drain_queue_batch(&session.logger);
    let ids: Vec<&str> = items.iter().map(|item| item.payload.id.as_str()).collect();
    let remaining = session
        .logger
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    json!({"drained": ids, "remaining": remaining})
}

fn config_step(input: &Input, session: &Session) -> Result<Value, Box<HelperError>> {
    let empty = Json::Object(IndexMap::new());
    let dynamic = input.dynamic.as_ref().unwrap_or(&empty);
    let key = config_key(&session.logger, dynamic).or_value_error()?;
    let sanitized = sanitize_config_key(&key);
    Ok(json!({"key": key, "sanitized": sanitized}))
}

fn group_step(session: &Session) -> Result<Value, Box<HelperError>> {
    let items = drain_queue_batch(&session.logger);
    let grouped = group_items_by_config(&session.logger, &items).or_value_error()?;
    let groups: Vec<Value> = grouped
        .iter()
        .map(|(key, group)| {
            let ids: Vec<&str> = group.iter().map(|item| item.payload.id.as_str()).collect();
            json!([key, ids])
        })
        .collect();
    Ok(json!({"groups": groups}))
}

fn ndjson_step(session: &Session) -> Result<Value, Box<HelperError>> {
    let items = drain_queue_batch(&session.logger);
    let text = combine_payloads_to_ndjson(&items).or_value_error()?;
    let lines = text
        .split('\n')
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    Ok(json!({"lines": lines, "trailing_newline": text.ends_with('\n')}))
}

async fn grouped_step(session: &Session) -> Value {
    let items = drain_queue_batch(&session.logger);
    let (sent, errors) = send_grouped_batch(&session.logger, &items).await;
    json!([sent, errors])
}

async fn send_step(session: &Session) -> Value {
    async_send_batch(&session.logger).await;
    json!({"uploads": uploads(&session.client)})
}

async fn retrieve_step(input: &Input, session: &Session) -> Result<Value, Box<HelperError>> {
    let Some(request_id) = &input.request_id else {
        return Err(failure("ValueError", "a retrieve step needs request_id"));
    };
    let start = input.start_time.map(timestamp).transpose()?;
    let payload = get_request_response_payload(&session.logger, request_id, start)
        .await
        .or_value_error()?;
    Ok(json!({"payload": payload}))
}

fn date_step(input: &Input) -> Result<Value, Box<HelperError>> {
    let Some(seconds) = input.timestamp else {
        return Err(failure("ValueError", "a date step needs timestamp"));
    };
    Ok(json!({"date": get_object_date(timestamp(seconds)?)}))
}

async fn flush_step(session: &Session) -> Value {
    let epoch = timestamp(0).unwrap_or_default();
    *session
        .logger
        .last_flush_time
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = epoch;
    flush_queue(&session.logger).await;
    let last = *session
        .logger
        .last_flush_time
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    json!({"updated": last > epoch, "uploads": uploads(&session.client)})
}

// the periodic case logs with a one second interval, so this outlasts the first scheduled flush
async fn wait_step(session: &Session) -> Value {
    tokio::time::sleep(Duration::from_millis(1500)).await;
    json!({"uploads": uploads(&session.client)})
}

fn timestamp(seconds: i64) -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::from_timestamp(seconds, 0)
        .ok_or_else(|| failure("ValueError", format!("invalid timestamp {seconds}")))
}
