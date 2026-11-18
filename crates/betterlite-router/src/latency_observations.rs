pub mod error;
pub mod event;
pub mod samples;
pub mod settings;

mod tests;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive as _;

use crate::cooldown_decisions::failure::FailureClass;
use crate::latency_sample_math;
use crate::routing_value_store::{Clock, RouteKey};

use self::error::LatencyError;
use self::event::{FailureEvent, SuccessEvent};
use self::samples::ModelGroupSamples;
use self::settings::Settings;

const BATCH_RETRIEVE_CALL_TYPES: [&str; 2] = ["aretrieve_batch", "retrieve_batch"];
const PENALTY_SECONDS: f64 = 1000.0_f64;
const ITEM_KIB: i64 = 1024;
const MINUTE_FORMAT: &str = "%Y-%m-%d-%H-%M";

// LowestLatencyLoggingHandler: the routing args it was built with, its clock, and the "{model_group}_map" cache
pub struct LatencyObservations {
    settings: Settings,
    clock: Clock,
    cache: Mutex<LocalTtlCache>,
}

// the values one success callback records, once the batch, window and channel checks pass
struct SuccessSample<'event> {
    key: RouteKey,
    deployment_id: &'event str,
    minute: String,
    value: f64,
    first_token: Option<f64>,
    total_tokens: i64,
}

impl LatencyObservations {
    pub fn new(settings: Settings, clock: Clock) -> Result<Self, LatencyError> {
        let cache = LocalTtlCache::with_clock(None, None, None, ITEM_KIB, Arc::clone(&clock))?;
        Ok(Self {
            settings,
            clock,
            cache: Mutex::new(cache),
        })
    }

    // log_success_event and async_log_success_event: one read-modify-write of the model group's map
    pub fn record_success(&self, event: &SuccessEvent<'_>) -> Result<(), LatencyError> {
        if event
            .call_type
            .is_some_and(|kind| BATCH_RETRIEVE_CALL_TYPES.contains(&kind))
        {
            return Ok(());
        }
        let limit = self.settings.max_latency_list_size;
        // a zero or negative size raises on the slice of a fresh deployment, and the callback then records nothing
        if limit <= 0 {
            return Ok(());
        }
        let (Some(model_group), Some(deployment_id)) =
            (event.channels.model_group(), event.deployment_id)
        else {
            return Ok(());
        };
        let elapsed = event
            .end_time
            .checked_sub(event.start_time)
            .ok_or(LatencyError::Time)?
            .to_f64()
            .ok_or(LatencyError::Time)?;
        let sample = SuccessSample {
            key: RouteKey::parse(&format!("{model_group}_map"))
                .map_err(|source| LatencyError::Key { source })?,
            deployment_id,
            minute: minute_key((self.clock)())?,
            value: event.usage.map_or(elapsed, |usage| {
                latency_sample_math::normalized(elapsed, usage.completion_tokens).unwrap_or(elapsed)
            }),
            first_token: first_token_seconds(event, elapsed)?,
            total_tokens: event.usage.map_or(0, |usage| usage.total_tokens),
        };
        self.store_success(&sample, limit)
    }

    // the success callback's locked read, update and write of the model group's map
    fn store_success(&self, sample: &SuccessSample<'_>, limit: i64) -> Result<(), LatencyError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let mut stored = read(&mut cache, &sample.key)?;
        let deployment = stored
            .deployments
            .entry(sample.deployment_id.to_owned())
            .or_default();
        push_window(&mut deployment.latency, sample.value, limit);
        if let Some(first_token) = sample.first_token {
            push_window(
                &mut deployment.time_to_first_token_seconds,
                first_token,
                limit,
            );
        }
        let usage = deployment.minutes.entry(sample.minute.clone()).or_default();
        usage.tpm = usage
            .tpm
            .checked_add(sample.total_tokens)
            .ok_or(LatencyError::Overflow)?;
        usage.rpm = usage.rpm.checked_add(1).ok_or(LatencyError::Overflow)?;
        write(&mut cache, &sample.key, &stored, self.settings.ttl_seconds)
    }

    // async_log_failure_event: a timeout appends the 1000-second penalty, every other failure is ignored
    pub fn record_failure(&self, event: &FailureEvent<'_>) -> Result<(), LatencyError> {
        let batch = event
            .call_type
            .is_some_and(|kind| BATCH_RETRIEVE_CALL_TYPES.contains(&kind));
        if batch || event.failure != FailureClass::Timeout {
            return Ok(());
        }
        let limit = self.settings.max_latency_list_size;
        // the penalty takes the same slice, so a zero or negative size records nothing here either
        if limit <= 0 {
            return Ok(());
        }
        let (Some(model_group), Some(deployment_id)) =
            (event.channels.model_group(), event.deployment_id)
        else {
            return Ok(());
        };
        let key = RouteKey::parse(&format!("{model_group}_map"))
            .map_err(|source| LatencyError::Key { source })?;
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let mut samples = read(&mut cache, &key)?;
        let deployment = samples
            .deployments
            .entry(deployment_id.to_owned())
            .or_default();
        push_window(&mut deployment.latency, PENALTY_SECONDS, limit);
        write(&mut cache, &key, &samples, self.settings.ttl_seconds)
    }

    // the reader B315's selection takes its windows and minute counters from
    pub fn samples(&self, model_group: &str) -> Result<ModelGroupSamples, LatencyError> {
        let key = RouteKey::parse(&format!("{model_group}_map"))
            .map_err(|source| LatencyError::Key { source })?;
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        read(&mut cache, &key)
    }
}

// _ttft_seconds: LiteLLM reads the logged first-token time only for a streaming request
fn first_token_seconds(
    event: &SuccessEvent<'_>,
    elapsed: f64,
) -> Result<Option<f64>, LatencyError> {
    if !event.stream {
        return Ok(None);
    }
    let Some(start) = event.completion_start_time else {
        return Ok(Some(elapsed));
    };
    let first_token = start
        .checked_sub(event.start_time)
        .ok_or(LatencyError::Time)?
        .to_f64()
        .ok_or(LatencyError::Time)?;
    Ok(Some(first_token))
}

fn minute_key(seconds: f64) -> Result<String, LatencyError> {
    let elapsed =
        Duration::try_from_secs_f64(seconds).map_err(|_source| LatencyError::Clock { seconds })?;
    let whole =
        i64::try_from(elapsed.as_secs()).map_err(|_source| LatencyError::Clock { seconds })?;
    let instant = DateTime::<Utc>::from_timestamp(whole, elapsed.subsec_nanos())
        .ok_or(LatencyError::Clock { seconds })?;
    Ok(instant.format(MINUTE_FORMAT).to_string())
}

// the window appends until it is full, then drops the oldest sample and keeps the newest
fn push_window(samples: &mut Vec<f64>, value: f64, limit: i64) {
    let room = usize::try_from(limit).unwrap_or(usize::MAX);
    if !samples.is_empty() && samples.len() >= room {
        samples.remove(0);
    }
    samples.push(value);
}

fn read(cache: &mut LocalTtlCache, key: &RouteKey) -> Result<ModelGroupSamples, LatencyError> {
    let Some(value) = cache.get(key.as_ref())? else {
        return Ok(ModelGroupSamples::default());
    };
    let CacheValue::Text(text) = value else {
        return Err(LatencyError::UnexpectedValue);
    };
    serde_json::from_str(&text).map_err(|source| LatencyError::Decoding { source })
}

// LocalTtlCache reads valid JSON text back as a value, so the sample text travels as a JSON string
fn write(
    cache: &mut LocalTtlCache,
    key: &RouteKey,
    samples: &ModelGroupSamples,
    ttl_seconds: f64,
) -> Result<(), LatencyError> {
    let text =
        serde_json::to_string(samples).map_err(|source| LatencyError::Encoding { source })?;
    let stored =
        serde_json::to_string(&text).map_err(|source| LatencyError::Encoding { source })?;
    cache.set(key.as_ref(), CacheValue::Text(stored), Some(ttl_seconds))?;
    Ok(())
}
