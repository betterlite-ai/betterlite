#![cfg(test)]

use std::error::Error;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_logging::finops::focus::destinations::factory::DestinationProvider;
use betterlite_logging::finops::focus::destinations::factory::error::FactoryError;
use betterlite_logging::finops::focus::destinations::factory::resolve::DestinationOverrides;
use betterlite_logging::finops::focus::engine::source::UsageSource;
use betterlite_logging::finops::focus::engine::{ExportError, ExportSettings, FocusExportEngine};
use betterlite_logging::focus_records::destinations::{FocusDestination, FocusTimeWindow};
use betterlite_logging::focus_records::transformer::UsageRow;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::finops::focus::export::FocusExporter;
use crate::finops::focus::support::{self, TestError, at};

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceCall {
    limit: Option<i64>,
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
}

#[derive(Clone)]
struct FakeSource {
    calls: Arc<Mutex<Vec<SourceCall>>>,
    fail: bool,
}

impl FakeSource {
    fn recording() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            fail: true,
        }
    }

    fn calls(&self) -> Vec<SourceCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl UsageSource for FakeSource {
    type Error = TestError;

    fn usage_data(
        &self,
        limit: Option<i64>,
        start_time_utc: Option<DateTime<Utc>>,
        end_time_utc: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<Vec<UsageRow>, TestError>> + Send {
        self.calls.lock().unwrap().push(SourceCall {
            limit,
            start: start_time_utc,
            end: end_time_utc,
        });
        if self.fail {
            return ready(Err(TestError));
        }
        ready(Ok(vec![usage_row()]))
    }
}

fn usage_row() -> UsageRow {
    UsageRow {
        date: String::from("2026-03-04"),
        api_key: String::from("api-key-hash"),
        spend: Decimal::new(125, 2),
        api_requests: 3,
        user_id: Some(String::from("user-1")),
        model: Some(String::from("gpt-4o")),
        model_group: Some(String::from("gpt-4o")),
        custom_llm_provider: Some(String::from("openai")),
        team_id: Some(String::from("team-1")),
        team_alias: Some(String::from("Team One")),
        organization_id: Some(String::from("org-1")),
        organization_alias: Some(String::from("Org One")),
        user_email: Some(String::from("user@example.com")),
        api_key_alias: Some(String::from("key-alias")),
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

fn engine<Source>(
    source: Source,
) -> FocusExportEngine<SilentDestination, Source, support::FixedClock>
where
    Source: UsageSource,
    Source::Error: Error + Send + Sync + 'static,
{
    FocusExportEngine::new(
        ExportSettings {
            provider: "s3".to_owned(),
            export_format: "csv".to_owned(),
            prefix: "focus_exports".to_owned(),
            destination_config: DestinationOverrides::default(),
        },
        SilentProvider,
        source,
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
    let engine = engine(FakeSource::failing());
    assert!(matches!(
        FocusExporter::export_all(&engine, None).await,
        Err(ExportError::Source(_))
    ));
    assert!(matches!(
        FocusExporter::export_window(&engine, &window, None).await,
        Err(ExportError::Source(_))
    ));
}

#[tokio::test]
async fn the_port_exports_all_data_with_no_bounds_and_the_caller_limit() {
    let source = FakeSource::recording();
    let engine = engine(source.clone());
    FocusExporter::export_all(&engine, Some(7))
        .await
        .expect("the fake export succeeds");
    assert_eq!(
        source.calls(),
        vec![SourceCall {
            limit: Some(7),
            start: None,
            end: None,
        }]
    );
}

#[tokio::test]
async fn the_port_exports_a_window_with_its_exact_bounds_and_the_caller_limit() {
    let source = FakeSource::recording();
    let engine = engine(source.clone());
    let window = FocusTimeWindow {
        start_time: at("2024-03-10T12:00:00Z"),
        end_time: at("2024-03-10T13:00:00Z"),
        frequency: "hourly".to_owned(),
    };
    FocusExporter::export_window(&engine, &window, Some(9))
        .await
        .expect("the fake export succeeds");
    assert_eq!(
        source.calls(),
        vec![SourceCall {
            limit: Some(9),
            start: Some(window.start_time),
            end: Some(window.end_time),
        }]
    );
}
