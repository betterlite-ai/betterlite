#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext};
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;

use crate::daily_records::DailyRecordError;
use crate::daily_records::callers::{DailySpendQueue, QueueFuture};
use crate::daily_records::projection::rows::base_spend_row;
use crate::daily_records::projection::{
    CallTypeEndpoints, DailyPayload, DailyPricing, DailyProjection, common_daily_transaction,
};
use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::records::BaseDailySpendTransaction;
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::DailySpendUpdate;

// The writer's state a projection reads: injected public rates, the calendar, and the route map B004 owns.
pub(super) struct Case {
    pub(super) prices: CasePrices,
    pub(super) zone: TimeZoneDatabase,
    pub(super) endpoints: CaseEndpoints,
}

impl Case {
    pub(super) fn with_prices(prices: BTreeMap<String, PriceRecord>) -> Self {
        Self {
            prices: CasePrices { prices },
            zone: TimeZoneDatabase::none(),
            endpoints: CaseEndpoints {
                routes: BTreeMap::new(),
            },
        }
    }

    pub(super) fn pricing(&self) -> DailyPricing<'_, CasePrices> {
        DailyPricing {
            context: &self.prices,
            zone: &self.zone,
            fallback_moment: DateTime::UNIX_EPOCH,
        }
    }
}

// The public rates a case injects: one exact "provider/model" record, and no deployment registry behind it.
pub(super) struct CasePrices {
    pub(super) prices: BTreeMap<String, PriceRecord>,
}

impl PriceLookup for CasePrices {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.prices
            .get(&price_key(model, custom_llm_provider))
            .cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.prices.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.effective_model_info(model, custom_llm_provider)
            .map(|record| PricedModel {
                record,
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            })
    }
}

impl SavingsPricingContext for CasePrices {
    fn resolve_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelIdentity> {
        Some(ModelIdentity {
            model: model.to_owned(),
            provider: custom_llm_provider.unwrap_or_default().to_owned(),
        })
    }

    fn deployment_price(
        &self,
        _deployment_id: Option<&str>,
        _model: &str,
    ) -> impl Future<Output = Option<PricedModel>> + Send {
        ready(None)
    }

    fn public_lookup(&self) -> &dyn PriceLookup {
        self
    }
}

// ROUTE_ENDPOINT_MAPPING as the caller supplies it: the singular image route is the mapping B004 owns.
pub(super) struct CaseEndpoints {
    pub(super) routes: BTreeMap<String, String>,
}

impl CallTypeEndpoints for CaseEndpoints {
    fn endpoint(&self, call_type: &str) -> Option<&str> {
        self.routes.get(call_type).map(String::as_str)
    }
}

// An owned daily sink: it records what a writer queued and fails the enqueue at the chosen position, like a full or closed queue.
pub(super) struct RecordingQueue {
    updates: Mutex<Vec<DailySpendUpdate>>,
    fail_at: Option<usize>,
}

impl RecordingQueue {
    pub(super) fn new(fail_at: Option<usize>) -> Self {
        Self {
            updates: Mutex::new(Vec::new()),
            fail_at,
        }
    }

    pub(super) fn recorded(&self) -> Vec<DailySpendUpdate> {
        self.updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl DailySpendQueue for RecordingQueue {
    fn enqueue(&self, update: DailySpendUpdate) -> QueueFuture<'_> {
        Box::pin(async move {
            let mut updates = self.updates.lock().unwrap_or_else(PoisonError::into_inner);
            if self.fail_at == Some(updates.len()) {
                return Err(SpendQueueError::ZeroCapacity);
            }
            updates.push(update);
            Ok(())
        })
    }
}

pub(super) fn daily_payload(text: &str) -> DailyPayload {
    serde_json::from_str(text).unwrap()
}

pub(super) fn price_record(text: &str) -> PriceRecord {
    serde_json::from_str(text).unwrap()
}

pub(super) fn price_key(model: &str, custom_llm_provider: Option<&str>) -> String {
    match custom_llm_provider.filter(|value| !value.is_empty()) {
        Some(provider) => format!("{provider}/{model}"),
        None => model.to_owned(),
    }
}

// The payload every projection needs to get past its key gates; a case that reads other fields writes its own text.
pub(super) const PAYLOAD_FIELDS: &str = r#""user":"u1","startTime":"2025-01-02T03:04:05Z","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","prompt_tokens":3,"completion_tokens":4,"spend":0.25"#;

pub(super) fn base_payload() -> String {
    format!("{{{PAYLOAD_FIELDS}}}")
}

pub(super) fn payload_with(fragment: &str) -> String {
    format!("{{{PAYLOAD_FIELDS},{fragment}}}")
}

pub(super) fn payload_with_metadata(metadata: &str) -> String {
    let quoted = serde_json::to_string(metadata).unwrap();
    payload_with(&format!("\"metadata\":{quoted}"))
}

// The source's common builder over one serialized payload, as every caller calls it.
pub(super) async fn project(
    case: &Case,
    text: &str,
    entity: DailySpendEntity,
) -> Result<Option<DailyProjection>, DailyRecordError> {
    let payload = daily_payload(text);
    common_daily_transaction(&payload, entity, &case.pricing(), &case.endpoints).await
}

// The base row a payload projects for the given entity.
pub(super) async fn projected_base(
    case: &Case,
    text: &str,
    entity: DailySpendEntity,
) -> BaseDailySpendTransaction {
    project(case, text, entity)
        .await
        .unwrap()
        .expect("the payload projects a row")
        .base
}

pub(super) async fn projected_row(case: &Case, text: &str, entity: DailySpendEntity) -> SpendRow {
    base_spend_row(&projected_base(case, text, entity).await)
}

pub(super) fn column(row: &SpendRow, name: &str) -> SpendColumnValue {
    row.get(name).cloned().unwrap()
}
