use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use betterlite_logging::mavvrik_delivery::api::MetricsMarker;
use chrono::{DateTime, Days, NaiveTime, Utc};
use tracing::warn;

use super::error::MavvrikError;
use super::export::{
    MavvrikEngineFactory, MavvrikExporter, MavvrikFocusLogger, is_empty_metrics_marker,
    parse_metrics_marker,
};

#[path = "scheduled_export/tests/catchup_behind_warning.rs"]
mod catchup_behind_warning;
#[path = "scheduled_export/tests/catchup_loop.rs"]
mod catchup_loop;
#[path = "scheduled_export/tests/catchup_start_date.rs"]
mod catchup_start_date;
#[path = "scheduled_export/tests/marker_check.rs"]
mod marker_check;
#[path = "scheduled_export/tests/register.rs"]
mod register;
#[path = "scheduled_export/tests/today_window.rs"]
mod today_window;
#[path = "scheduled_export/tests/yesterday.rs"]
mod yesterday;

// _MAX_CATCHUP_DAYS: the oldest date a single run re-exports, as a count of days ending at yesterday
pub(super) const MAX_CATCHUP_DAYS: u64 = 7;

// _run_scheduled_export: read the marker, re-export every missed date still inside seven days, then yesterday
pub async fn run_scheduled_export<F>(
    logger: &MavvrikFocusLogger<F>,
) -> Result<(), MavvrikError<<F::Engine as MavvrikExporter>::Error>>
where
    F: MavvrikEngineFactory,
{
    let engine = logger.ensure_engine().map_err(MavvrikError::Export)?;
    let marker = engine
        .metrics_marker()
        .await
        .map_err(MavvrikError::Export)?;
    let now = logger.clock.now();
    let yesterday = now
        .date_naive()
        .and_time(NaiveTime::MIN)
        .and_utc()
        .checked_sub_days(Days::new(1))
        .ok_or(MavvrikError::TimeWindowOutOfRange)?;
    let last_ingested = parse_metrics_marker(&marker);
    if catch_up_needed(&marker, last_ingested, yesterday) {
        let earliest = yesterday
            .checked_sub_days(Days::new(MAX_CATCHUP_DAYS - 1))
            .ok_or(MavvrikError::TimeWindowOutOfRange)?;
        let start =
            catch_up_start(last_ingested, earliest).ok_or(MavvrikError::TimeWindowOutOfRange)?;
        warn_when_behind(last_ingested, earliest, start);
        export_catch_up(logger, start, yesterday, now).await?;
    }
    let window = FocusTimeWindow {
        start_time: yesterday,
        end_time: now,
        frequency: logger.frequency.clone(),
    };
    logger.export_window(&window, None).await
}

// an empty marker catches up everything, a parsed one only before yesterday, a bad one never; <= leaves the same dates, hence mutants::skip
#[cfg_attr(test, mutants::skip)]
fn catch_up_needed(
    marker: &MetricsMarker,
    last_ingested: Option<DateTime<Utc>>,
    yesterday: DateTime<Utc>,
) -> bool {
    if is_empty_metrics_marker(marker) {
        return true;
    }
    last_ingested.is_some_and(|last| last < yesterday)
}

// the first missed date: the day after the marker, but not before the earliest date the seven-day window reaches
fn catch_up_start(
    last_ingested: Option<DateTime<Utc>>,
    earliest: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    last_ingested.map_or(Some(earliest), |last| {
        last.checked_add_days(Days::new(1))
            .map(|next| next.max(earliest))
    })
}

// a marker more than seven days behind: the warning names where it stands and where the re-export starts
fn warn_when_behind(
    last_ingested: Option<DateTime<Utc>>,
    earliest: DateTime<Utc>,
    start: DateTime<Utc>,
) {
    let Some(marker_date) = last_ingested else {
        return;
    };
    let Some(next) = marker_date.checked_add_days(Days::new(1)) else {
        return;
    };
    if next < earliest {
        warn!(
            "Mavvrik FOCUS export: metricsMarker is more than {MAX_CATCHUP_DAYS} days behind ({}). Catching up from {} only; earlier data will not be re-exported.",
            marker_date.date_naive(),
            start.date_naive()
        );
    }
}

// one daily window per missed date, ended at the date's own midnight or at now while the date is still running
async fn export_catch_up<F>(
    logger: &MavvrikFocusLogger<F>,
    start: DateTime<Utc>,
    yesterday: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), MavvrikError<<F::Engine as MavvrikExporter>::Error>>
where
    F: MavvrikEngineFactory,
{
    let mut catch_up = start;
    while catch_up < yesterday {
        let next = catch_up
            .checked_add_days(Days::new(1))
            .ok_or(MavvrikError::TimeWindowOutOfRange)?;
        tracing::info!(
            "Mavvrik FOCUS export: catching up missed date {}",
            catch_up.date_naive()
        );
        let window = FocusTimeWindow {
            start_time: catch_up,
            end_time: next.min(now),
            frequency: logger.frequency.clone(),
        };
        logger.export_window(&window, None).await?;
        catch_up = next;
    }
    Ok(())
}
