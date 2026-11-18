#![cfg(test)]

use std::future::{Future, ready};

use betterlite_logging::finops::focus::destinations::factory::DestinationProvider;
use betterlite_logging::finops::focus::destinations::factory::error::FactoryError;
use betterlite_logging::finops::focus::destinations::factory::resolve::DestinationOverrides;
use betterlite_logging::finops::focus::engine::source::UsageSource;
use betterlite_logging::finops::focus::engine::{ExportError, ExportSettings, FocusExportEngine};
use betterlite_logging::focus_records::destinations::{FocusDestination, FocusTimeWindow};
use betterlite_logging::focus_records::transformer::UsageRow;
use chrono::{DateTime, Utc};

use crate::finops::focus::export::FocusExporter;
use crate::finops::focus::support::{self, TestError, at};

struct FailingSource;

impl UsageSource for FailingSource {
    type Error = TestError;

    fn usage_data(
        &self,
        _limit: Option<i64>,
        _start_time_utc: Option<DateTime<Utc>>,
        _end_time_utc: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<Vec<UsageRow>, TestError>> + Send {
        ready(Err(TestError))
    }
}

struct SilentDestination;

impl FocusDestination for SilentDestination {
    type Error = TestError;

    fn deliver<'delivery>(
        &'delivery self,
        _content: &'delivery [u8],
        _time_window: &'delivery FocusTimeWindow,
        _filename: &'delivery str,
    ) -> impl Future<Output = Result<(), TestError>> + Send + 'delivery {
        ready(Ok(()))
    }
}

struct SilentProvider;

impl DestinationProvider for SilentProvider {
    type Destination = SilentDestination;

    fn create(
        self,
        _provider: &str,
        _prefix: &str,
        _overrides: &DestinationOverrides,
    ) -> Result<SilentDestination, FactoryError> {
        Ok(SilentDestination)
    }
}

fn engine() -> FocusExportEngine<SilentDestination, FailingSource, support::FixedClock> {
    FocusExportEngine::new(
        ExportSettings {
            provider: "s3".to_owned(),
            export_format: "csv".to_owned(),
            prefix: "focus_exports".to_owned(),
            destination_config: DestinationOverrides::default(),
        },
        SilentProvider,
        FailingSource,
        support::FixedClock {
            now: at(support::FIXED_NOW),
        },
    )
    .expect("the fake destination builds")
}

#[tokio::test]
async fn the_port_keeps_the_engine_error_instead_of_reporting_success() {
    let window = FocusTimeWindow {
        start_time: at("2024-03-10T12:00:00Z"),
        end_time: at("2024-03-10T13:00:00Z"),
        frequency: "hourly".to_owned(),
    };
    let engine = engine();
    assert!(matches!(
        FocusExporter::export_all(&engine, None).await,
        Err(ExportError::Source(_))
    ));
    assert!(matches!(
        FocusExporter::export_window(&engine, &window, None).await,
        Err(ExportError::Source(_))
    ));
}
