use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::cooldown_policy::event::{
    CooldownEvent, CooldownObservationError, CooldownObserver,
};
use betterlite_router::cooldown_state::record::CooldownRecord;
use betterlite_router::cooldown_state::shared::SharedCooldowns;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::routing_read_batch::store::{BatchRead, BatchReadFuture};
use betterlite_router::routing_value_store::RouteKey;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use super::{ActiveOutput, DeploymentInput, EventOutput, OutageOutput};

pub(super) fn fixed(value: f64) -> String {
    format!("{value:.6}")
}

pub(super) fn seconds(millis: u64) -> Result<f64, String> {
    Decimal::from(millis)
        .checked_div(Decimal::from(1_000_i64))
        .and_then(|value| value.to_f64())
        .ok_or_else(|| format!("{millis} milliseconds cannot be represented as seconds"))
}

pub(super) fn spec(input: &DeploymentInput) -> Result<DeploymentSpec, String> {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(input.model.clone()));
    params.insert("api_key".to_owned(), Json::Text("sk-probe".to_owned()));
    if let Some(api_base) = &input.api_base {
        params.insert("api_base".to_owned(), Json::Text(api_base.clone()));
    }
    if let Some(provider) = &input.custom_llm_provider {
        params.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(provider.clone()),
        );
    }
    if let Some(time) = input.cooldown_time {
        params.insert("cooldown_time".to_owned(), json_number(time.0)?);
    }
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(input.id.clone()));
    if let Some(allowed) = input.model_info.allowed_fails {
        info.insert(
            "allowed_fails".to_owned(),
            Json::Number(Number::from(allowed)),
        );
    }
    if let Some(time) = input.model_info.cooldown_time {
        info.insert("cooldown_time".to_owned(), json_number(time.0)?);
    }
    DeploymentSpec::new(
        input.model_name.clone(),
        params,
        info,
        JsonObject::new(),
        generated_id,
    )
    .map_err(|error| error.to_string())
}

fn json_number(value: f64) -> Result<Json, String> {
    let number = Number::from_f64(value).ok_or_else(|| format!("{value} is not a number"))?;
    Ok(Json::Number(number))
}

fn generated_id() -> String {
    "generated".to_owned()
}

// the two Prometheus calls router_cooldown_event_callback makes, recorded from the one typed observer hook
#[derive(Default)]
pub(super) struct Recorder {
    events: Mutex<Vec<EventOutput>>,
    outages: Mutex<Vec<OutageOutput>>,
}

impl Recorder {
    pub(super) fn events(&self) -> Vec<EventOutput> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn outages(&self) -> Vec<OutageOutput> {
        self.outages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CooldownObserver for Recorder {
    fn deployment_cooled_down(
        &self,
        event: &CooldownEvent<'_>,
    ) -> Result<(), CooldownObservationError> {
        self.outages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(OutageOutput {
                model_name: event.model_name.to_owned(),
                model_id: event.model_id.to_owned(),
                api_base: event.api_base.to_owned(),
                provider: event.provider.to_owned(),
            });
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(EventOutput {
                model_name: event.model_name.to_owned(),
                model_id: event.model_id.to_owned(),
                api_base: event.api_base.to_owned(),
                provider: event.provider.to_owned(),
                exception_status: event.exception_status.to_owned(),
            });
        Ok(())
    }
}

pub(super) fn active_output(
    model_id: String,
    status_code: String,
    timestamp: f64,
    cooldown_time: f64,
) -> ActiveOutput {
    ActiveOutput {
        model_id,
        status_code,
        timestamp: fixed(timestamp),
        cooldown_time: fixed(cooldown_time),
    }
}

#[derive(Default)]
pub(super) struct FakeCooldowns {
    records: Mutex<BTreeMap<RouteKey, CooldownRecord>>,
}

impl SharedCooldowns for FakeCooldowns {
    fn batch_read<'store>(
        &'store self,
        keys: &'store [RouteKey],
    ) -> BatchReadFuture<'store, CooldownRecord> {
        Box::pin(async move {
            let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
            let values = keys.iter().map(|key| records.get(key).cloned()).collect();
            BatchRead {
                values,
                warning: None,
            }
        })
    }

    fn write(&self, key: &RouteKey, record: &CooldownRecord, _ttl_seconds: f64) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), record.clone());
    }
}
