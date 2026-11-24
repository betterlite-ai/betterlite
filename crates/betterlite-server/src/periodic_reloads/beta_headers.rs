use std::collections::BTreeMap;
use std::error::Error;

use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde::Deserialize;

use crate::periodic_reloads::config_rows::ReloadConfigRows;
use crate::periodic_reloads::sources::ReloadSources;
use crate::periodic_reloads::state::PeriodicReloads;

const INTERVAL_HOURS_FIELD: &str = "interval_hours";
const FORCE_RELOAD_FIELD: &str = "force_reload";
const PROVIDER_ALIASES_KEY: &str = "provider_aliases";
const DESCRIPTION_KEY: &str = "description";
const SECONDS_PER_HOUR: u32 = 3_600;
const HOURS_DECIMAL_PLACES: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum BetaReloadError<RowsError>
where
    RowsError: Error + Send + Sync + 'static,
{
    #[error("the anthropic beta headers reload config could not be serialized: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the anthropic beta headers reload config has a field of the wrong type: {source}")]
    Decode {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the anthropic beta headers reload state could not be read or written: {source}")]
    Rows {
        #[from]
        source: RowsError,
    },
}

#[derive(Deserialize)]
struct StoredBetaReloadConfig {
    #[serde(default)]
    interval_hours: Option<u32>,
    #[serde(default)]
    force_reload: Option<bool>,
}

struct BetaReloadConfig {
    interval_hours: Option<u32>,
    force_reload: bool,
}

pub(crate) async fn reload<Rows>(
    db: &Rows,
    reloads: &PeriodicReloads,
    sources: &ReloadSources,
    now: DateTime<Utc>,
) -> Result<(), BetaReloadError<Rows::Error>>
where
    Rows: ReloadConfigRows,
{
    let Some(row) = db.read_beta_headers().await? else {
        return Ok(());
    };
    let Some(config) = decode_beta_reload_config(&row.param_value)? else {
        return Ok(());
    };
    if config.interval_hours.is_none() && !config.force_reload {
        return Ok(());
    }
    if !reload_is_due(reloads, &config, now) {
        return Ok(());
    }
    perform_reload(db, reloads, sources, &config, now).await
}

fn decode_beta_reload_config<RowsError>(
    value: &JsonData,
) -> Result<Option<BetaReloadConfig>, BetaReloadError<RowsError>>
where
    RowsError: Error + Send + Sync + 'static,
{
    if matches!(value, JsonData::Null) {
        return Ok(None);
    }
    let text = serde_json::to_string(value).map_err(|source| BetaReloadError::Encode { source })?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    let stored: StoredBetaReloadConfig = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| BetaReloadError::Decode { source })?;
    // the text was just serialized from one value, so there is no trailing input to reject
    Ok(Some(BetaReloadConfig {
        interval_hours: stored.interval_hours,
        force_reload: stored.force_reload.unwrap_or(false),
    }))
}

fn reload_is_due(reloads: &PeriodicReloads, config: &BetaReloadConfig, now: DateTime<Utc>) -> bool {
    if config.force_reload {
        tracing::info!("Anthropic beta headers reload triggered by force reload flag");
        return true;
    }
    let Some(interval_hours) = config.interval_hours else {
        return false;
    };
    let Some(last_reload) = reloads.last_beta_headers_reload() else {
        tracing::info!(
            "Anthropic beta headers reload triggered - no previous reload time recorded"
        );
        return true;
    };
    let interval = TimeDelta::try_hours(i64::from(interval_hours)).unwrap_or(TimeDelta::MAX);
    let elapsed = now.signed_duration_since(last_reload);
    if elapsed < interval {
        return false;
    }
    let hours = hours_since(elapsed);
    tracing::info!(
        "Anthropic beta headers reload triggered by interval. Hours since last reload: {hours:.2}, Interval: {interval_hours}"
    );
    true
}

async fn perform_reload<Rows>(
    db: &Rows,
    reloads: &PeriodicReloads,
    sources: &ReloadSources,
    config: &BetaReloadConfig,
    now: DateTime<Utc>,
) -> Result<(), BetaReloadError<Rows::Error>>
where
    Rows: ReloadConfigRows,
{
    let matrix = sources.beta_headers.reload().await;
    reloads.set_last_beta_headers_reload(now);
    db.write_beta_headers(encode_beta_reload_config(config.interval_hours))
        .await?;
    db.evict_beta_headers().await?;
    let providers = matrix
        .0
        .keys()
        .filter(|key| key.as_str() != PROVIDER_ALIASES_KEY && key.as_str() != DESCRIPTION_KEY)
        .count();
    tracing::info!("Anthropic beta headers config reloaded successfully. Providers: {providers}");
    Ok(())
}

fn encode_beta_reload_config(interval_hours: Option<u32>) -> JsonData {
    let mut fields = BTreeMap::new();
    let interval = interval_hours.map_or(JsonData::Null, |hours| {
        JsonData::Number(serde_json::Number::from(hours))
    });
    fields.insert(INTERVAL_HOURS_FIELD.to_owned(), interval);
    fields.insert(FORCE_RELOAD_FIELD.to_owned(), JsonData::Bool(false));
    JsonData::Object(fields)
}

fn hours_since(elapsed: TimeDelta) -> Decimal {
    let Some(hours) =
        Decimal::from(elapsed.num_seconds()).checked_div(Decimal::from(SECONDS_PER_HOUR))
    else {
        return Decimal::ZERO;
    };
    hours.round_dp_with_strategy(HOURS_DECIMAL_PLACES, RoundingStrategy::MidpointNearestEven)
}
