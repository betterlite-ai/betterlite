use std::sync::Arc;

use indexmap::IndexMap;
use jiff::tz::TimeZone;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Serialize;

use super::settings::CostManagementSettings;
use super::tags::extract_tags;
use crate::fields::payload::StandardLoggingPayload;

pub const BILLING_CURRENCY: &str = "USD";
const UNKNOWN: &str = "unknown";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CostKey {
    pub provider: String,
    pub model: String,
    pub period: String,
    pub tags: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FocusCostEntry {
    #[serde(rename = "ProviderName")]
    pub provider_name: String,
    #[serde(rename = "ChargeDescription")]
    pub charge_description: String,
    #[serde(rename = "ChargePeriodStart")]
    pub charge_period_start: String,
    #[serde(rename = "ChargePeriodEnd")]
    pub charge_period_end: String,
    #[serde(
        rename = "BilledCost",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub billed_cost: Decimal,
    #[serde(rename = "BillingCurrency")]
    pub billing_currency: String,
    #[serde(rename = "Tags")]
    pub tags: IndexMap<String, String>,
}

#[must_use]
pub fn aggregate(
    logs: &[Arc<StandardLoggingPayload>],
    settings: &CostManagementSettings,
    zone: &TimeZone,
) -> Vec<FocusCostEntry> {
    let mut entries: IndexMap<CostKey, FocusCostEntry> = IndexMap::new();
    for payload in logs {
        if payload.response_cost == Decimal::ZERO {
            continue;
        }
        let Some(period) = period_of(zone, payload.start_time) else {
            tracing::warn!(
                start_time = %payload.start_time,
                "Datadog Cost Management: the request start time is out of range; dropping the record"
            );
            continue;
        };
        add_payload(&mut entries, payload, settings, &period);
    }
    entries.into_values().collect()
}

fn add_payload(
    entries: &mut IndexMap<CostKey, FocusCostEntry>,
    payload: &StandardLoggingPayload,
    settings: &CostManagementSettings,
    period: &str,
) {
    let tags = extract_tags(&settings.environment, &settings.cost_tag_keys, payload);
    let provider = payload
        .custom_llm_provider
        .clone()
        .unwrap_or_else(|| UNKNOWN.to_owned());
    let model = payload.model.clone();
    let mut ordered: Vec<(String, String)> = tags
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    ordered.sort();
    let key = CostKey {
        provider: provider.clone(),
        model: model.clone(),
        period: period.to_owned(),
        tags: ordered,
    };
    let entry = entries.entry(key).or_insert_with(|| FocusCostEntry {
        provider_name: provider,
        charge_description: format!("LLM Usage for {model}"),
        charge_period_start: period.to_owned(),
        charge_period_end: period.to_owned(),
        billed_cost: Decimal::ZERO,
        billing_currency: BILLING_CURRENCY.to_owned(),
        tags,
    });
    if let Some(total) = entry.billed_cost.checked_add(payload.response_cost) {
        entry.billed_cost = total;
    } else {
        tracing::warn!(
            cost = %payload.response_cost,
            "Datadog Cost Management: the cost total overflowed; dropping this record's cost"
        );
    }
}

#[must_use]
pub fn period_of(zone: &TimeZone, timestamp: Decimal) -> Option<String> {
    let rounded = timestamp.round_dp(6);
    let seconds = rounded.floor().to_i64()?;
    let moment = jiff::Timestamp::from_second(seconds).ok()?;
    Some(zone.to_datetime(moment).date().to_string())
}
