use betterlite_auth::budget_primitives::window_start::coerce_datetime;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_roi::records::{ROIReport, ROISyncStatus};
use betterlite_roi::settings::ROISettings;
use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::roi_settings::deployments::router_estimator_models;
use crate::roi_settings::public::public_settings;
use crate::roi_settings::store::load_settings;

use super::error::RoiRoutesError;
use super::ports::RouteSyncRun;
use super::reports::load_report;
use super::state::RoiRouteState;

const MICROS_PER_MINUTE: i64 = 60_000_000;

pub fn next_update(
    settings: &ROISettings,
    status: &ROISyncStatus,
    report: Option<&ROIReport>,
) -> Result<Option<DateTime<Utc>>, RoiRoutesError> {
    let Some(report) = report else {
        return Ok(None);
    };
    if settings.repos.is_empty()
        || settings.estimator_model.is_empty()
        || settings.update_interval_minutes.abs().to_bits() == 0
        || status.running
    {
        return Ok(None);
    }
    let anchor = status
        .finished_at
        .as_deref()
        .filter(|text| !text.is_empty())
        .or_else(|| status.started_at.as_deref().filter(|text| !text.is_empty()))
        .unwrap_or(&report.synced_at);
    let parsed = coerce_datetime(Some(&JsonData::String(anchor.to_owned()))).ok_or_else(|| {
        RoiRoutesError::Timestamp {
            value: anchor.to_owned(),
        }
    })?;
    let delta = interval_delta(settings.update_interval_minutes)?;
    parsed
        .with_timezone(&Utc)
        .checked_add_signed(delta)
        .map(Some)
        .ok_or(RoiRoutesError::TimestampOverflow)
}

fn interval_delta(minutes: f64) -> Result<TimeDelta, RoiRoutesError> {
    let value = Decimal::try_from(minutes).map_err(|_source| RoiRoutesError::TimestampOverflow)?;
    let micros = value
        .checked_mul(Decimal::from(MICROS_PER_MINUTE))
        .ok_or(RoiRoutesError::TimestampOverflow)?
        .round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven);
    let micros = i64::try_from(micros).map_err(|_source| RoiRoutesError::TimestampOverflow)?;
    Ok(TimeDelta::microseconds(micros))
}

pub async fn run_scheduled_sync(state: &RoiRouteState) -> Result<(), RoiRoutesError> {
    let Some(repository) = state.repository.clone() else {
        return Ok(());
    };
    let settings = load_settings(repository.as_ref(), state.cipher.as_ref()).await?;
    if settings.update_interval_minutes.abs().to_bits() == 0 {
        return Ok(());
    }
    if !public_settings(&settings, state.catalog.as_deref()).ready {
        return Ok(());
    }
    let store = state.sync_store.store().await?;
    let status = match store.read_status().await? {
        Some(status) => status,
        None => state.manager.current_status(),
    };
    let report = load_report(repository.as_ref()).await?;
    let Some(next) = next_update(&settings, &status, report.as_ref())? else {
        return Ok(());
    };
    if next > (state.clock)() {
        return Ok(());
    }
    let complete = (state.completion_caller)(&settings)?;
    let estimator_models =
        router_estimator_models(state.catalog.as_deref(), &settings.estimator_model);
    let spend_reader = (state.spend_reader)();
    let interval = settings.update_interval_minutes;
    let run = RouteSyncRun {
        settings,
        repository,
        spend_reader,
        complete,
        estimator_models,
        store: Some(store),
        scheduled_interval: interval,
    };
    let started = state.manager.start_run(run).await;
    if !started {
        tracing::debug!("the scheduled ROI sync did not start");
    }
    Ok(())
}
