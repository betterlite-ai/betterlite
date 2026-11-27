use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_logging::prometheus::budget_gauges::gauge::EntityBudgetGauges;
use betterlite_logging::prometheus::budget_gauges::inputs::KeyBudgetGaugeInput;
use betterlite_logging::prometheus::budget_gauges::lists::key_list::{
    KeyListEntry as BudgetKeyEntry, set_key_list_budget_metrics,
};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::RemovableMetric;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_prometheus::collector_factory::collector::Collector;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::key_query_plan::response::KeyListEntry as KeyPageEntry;

use super::page_walk::{EntityPageWalk, initialize_entity};
use super::source::{BudgetPageError, BudgetPageSource, Page};

mod tests;

// UserAPIKeyAuth keeps its fields private, so the page row is read through the serde form LiteLLM dumps
#[derive(Deserialize)]
struct KeyAuthFields {
    #[serde(default)]
    token: Option<SecretString>,
    #[serde(default)]
    key_alias: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision")]
    spend: Decimal,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    max_budget: Option<Decimal>,
    #[serde(default)]
    budget_reset_at: Option<Timestamp>,
}

struct KeyWalk<'walk, Gauge>
where
    Gauge: Collector + RemovableMetric,
{
    source: &'walk dyn BudgetPageSource,
    gauges: &'walk EntityBudgetGauges<Gauge>,
    labels: LabelSource<'walk>,
    now: DateTime<Utc>,
}

#[async_trait]
impl<Gauge> EntityPageWalk for KeyWalk<'_, Gauge>
where
    Gauge: Collector + RemovableMetric + Send + Sync,
    Gauge::Error: Send + Sync,
{
    type Row = KeyPageEntry;

    async fn fetch(&self, page_size: u64, page: u64) -> Result<Page<Self::Row>, BudgetPageError> {
        self.source.keys_page(page_size, page).await
    }

    fn apply(&self, rows: Vec<Self::Row>) -> Result<(), BudgetPageError> {
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(entry) = budget_entry(row)? {
                entries.push(entry);
            }
        }
        set_key_list_budget_metrics(self.gauges, &entries, self.labels, self.now).map_err(|error| {
            BudgetPageError::Gauge {
                source: Box::new(error),
            }
        })
    }
}

// LiteLLM's _set_key_list_budget_metrics sets metrics only for the full UserAPIKeyAuth records
fn budget_entry(entry: KeyPageEntry) -> Result<Option<BudgetKeyEntry>, BudgetPageError> {
    match entry {
        KeyPageEntry::Token(token) => Ok(Some(BudgetKeyEntry::Token(token))),
        KeyPageEntry::Auth(auth) => Ok(Some(BudgetKeyEntry::Record(key_record(&auth)?))),
        KeyPageEntry::Deleted(_) => Ok(None),
    }
}

fn key_record(auth: &UserAPIKeyAuth) -> Result<KeyBudgetGaugeInput, BudgetPageError> {
    let fields: KeyAuthFields = serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .map_err(BudgetPageError::Row)?;
    let token = fields.token.map(|secret| secret.expose_secret().to_owned());
    Ok(KeyBudgetGaugeInput {
        token: token.unwrap_or_default(),
        key_alias: fields.key_alias,
        spend: Some(fields.spend),
        max_budget: fields.max_budget,
        budget_reset_at: fields.budget_reset_at.map(Timestamp::utc),
    })
}

pub async fn refresh_keys<Gauge>(
    source: Option<&dyn BudgetPageSource>,
    gauges: &EntityBudgetGauges<Gauge>,
    labels: LabelSource<'_>,
    now: DateTime<Utc>,
) where
    Gauge: Collector + RemovableMetric + Send + Sync,
    Gauge::Error: Send + Sync,
{
    let Some(source) = source else {
        tracing::debug!("Prometheus: skipping key metrics initialization, DB not initialized");
        return;
    };
    initialize_entity(
        &KeyWalk {
            source,
            gauges,
            labels,
            now,
        },
        "keys",
    )
    .await;
}
