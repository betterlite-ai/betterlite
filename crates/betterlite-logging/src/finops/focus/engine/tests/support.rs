use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::finops::focus::destinations::factory::DestinationProvider;
use crate::finops::focus::destinations::factory::error::FactoryError;
use crate::finops::focus::destinations::factory::resolve::DestinationOverrides;
use crate::finops::focus::engine::source::UsageSource;
use crate::finops::focus::engine::{ExportSettings, FocusExportEngine, format_for};
use crate::finops::metering::clock::UtcClock;
use crate::focus_records::destinations::{FocusDestination, FocusTimeWindow};
use crate::focus_records::table::FocusTable;
use crate::focus_records::transformer::{TransformError, UsageRow};

pub(super) const PREFIX: &str = "focus-exports";

#[derive(Debug, thiserror::Error)]
#[error("the recording source failed")]
pub(super) struct SourceFailed;

#[derive(Debug, thiserror::Error)]
#[error("the recording destination failed")]
pub(super) struct DeliveryFailed;

#[derive(Clone, Debug)]
pub(super) struct Delivery {
    pub(super) content: Vec<u8>,
    pub(super) window: FocusTimeWindow,
    pub(super) filename: String,
}

#[derive(Clone, Default)]
pub(super) struct RecordingDestination {
    deliveries: Arc<Mutex<Vec<Delivery>>>,
}

impl RecordingDestination {
    pub(super) fn deliveries(&self) -> Vec<Delivery> {
        self.deliveries
            .lock()
            .expect("the deliveries mutex is not poisoned")
            .clone()
    }
}

impl FocusDestination for RecordingDestination {
    type Error = DeliveryFailed;

    fn deliver<'delivery>(
        &'delivery self,
        content: &'delivery [u8],
        time_window: &'delivery FocusTimeWindow,
        filename: &'delivery str,
    ) -> impl Future<Output = Result<(), DeliveryFailed>> + Send + 'delivery {
        self.deliveries
            .lock()
            .expect("the deliveries mutex is not poisoned")
            .push(Delivery {
                content: content.to_vec(),
                window: time_window.clone(),
                filename: filename.to_owned(),
            });
        ready(Ok(()))
    }
}

#[derive(Clone, Debug)]
pub(super) struct SourceCall {
    pub(super) limit: Option<i64>,
    pub(super) start_time_utc: Option<DateTime<Utc>>,
    pub(super) end_time_utc: Option<DateTime<Utc>>,
}

#[derive(Clone, Default)]
pub(super) struct RecordingSource {
    calls: Arc<Mutex<Vec<SourceCall>>>,
    rows: Vec<UsageRow>,
}

impl RecordingSource {
    pub(super) fn calls(&self) -> Vec<SourceCall> {
        self.calls
            .lock()
            .expect("the source calls mutex is not poisoned")
            .clone()
    }
}

impl UsageSource for RecordingSource {
    type Error = SourceFailed;

    fn usage_data(
        &self,
        limit: Option<i64>,
        start_time_utc: Option<DateTime<Utc>>,
        end_time_utc: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<Vec<UsageRow>, SourceFailed>> + Send {
        self.calls
            .lock()
            .expect("the source calls mutex is not poisoned")
            .push(SourceCall {
                limit,
                start_time_utc,
                end_time_utc,
            });
        ready(Ok(self.rows.clone()))
    }
}

#[derive(Clone, Debug)]
pub(super) struct CreateCall {
    pub(super) provider: String,
    pub(super) prefix: String,
    pub(super) overrides: DestinationOverrides,
}

#[derive(Clone, Default)]
pub(super) struct RecordingFactory {
    calls: Arc<Mutex<Vec<CreateCall>>>,
    destination: RecordingDestination,
}

impl RecordingFactory {
    pub(super) fn calls(&self) -> Vec<CreateCall> {
        self.calls
            .lock()
            .expect("the factory calls mutex is not poisoned")
            .clone()
    }
}

impl DestinationProvider for RecordingFactory {
    type Destination = RecordingDestination;

    fn create(
        self,
        provider: &str,
        prefix: &str,
        overrides: &DestinationOverrides,
    ) -> Result<RecordingDestination, FactoryError> {
        self.calls
            .lock()
            .expect("the factory calls mutex is not poisoned")
            .push(CreateCall {
                provider: provider.to_owned(),
                prefix: prefix.to_owned(),
                overrides: overrides.clone(),
            });
        Ok(self.destination)
    }
}

pub(super) struct FailingFactory;

impl DestinationProvider for FailingFactory {
    type Destination = RecordingDestination;

    fn create(
        self,
        _provider: &str,
        _prefix: &str,
        _overrides: &DestinationOverrides,
    ) -> Result<RecordingDestination, FactoryError> {
        Err(FactoryError::MissingS3BucketName)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FixedClock {
    pub(super) now: DateTime<Utc>,
}

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

pub(super) fn refusing_transform(_rows: &[UsageRow]) -> Result<FocusTable, TransformError> {
    Err(TransformError::Overflow {
        field: "spend",
        value: Decimal::MAX,
    })
}

pub(super) fn empty_transform(_rows: &[UsageRow]) -> Result<FocusTable, TransformError> {
    Ok(FocusTable {
        rows: 0,
        columns: Vec::new(),
    })
}

pub(super) fn timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("the test timestamp is valid RFC 3339")
        .with_timezone(&Utc)
}

pub(super) fn window() -> FocusTimeWindow {
    FocusTimeWindow {
        start_time: timestamp("2026-01-02T03:04:05Z"),
        end_time: timestamp("2026-01-02T04:05:06Z"),
        frequency: String::from("daily"),
    }
}

pub(super) fn settings(export_format: &str) -> ExportSettings {
    ExportSettings {
        provider: String::from("s3"),
        export_format: export_format.to_owned(),
        prefix: String::from(PREFIX),
        destination_config: DestinationOverrides::default(),
    }
}

pub(super) fn usage_row() -> UsageRow {
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

pub(super) fn data_row(fields: &[(&str, JsonData)]) -> BTreeMap<String, JsonData> {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn number(value: &str) -> JsonData {
    serde_json::from_str(value).expect("the test number is valid JSON")
}

pub(super) fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

pub(super) struct Harness {
    pub(super) engine: FocusExportEngine<RecordingDestination, RecordingSource, FixedClock>,
    destination: RecordingDestination,
    source: RecordingSource,
}

impl Harness {
    pub(super) fn deliveries(&self) -> Vec<Delivery> {
        self.destination.deliveries()
    }

    pub(super) fn source_calls(&self) -> Vec<SourceCall> {
        self.source.calls()
    }
}

pub(super) fn harness(export_format: &str, rows: Vec<UsageRow>, now: DateTime<Utc>) -> Harness {
    let destination = RecordingDestination::default();
    let source = RecordingSource {
        rows,
        ..RecordingSource::default()
    };
    let factory = RecordingFactory {
        destination: destination.clone(),
        ..RecordingFactory::default()
    };
    let engine = FocusExportEngine::new(
        settings(export_format),
        factory,
        source.clone(),
        FixedClock { now },
    )
    .expect("the recording factory builds a destination");
    Harness {
        engine,
        destination,
        source,
    }
}

pub(super) fn harness_with_transform(
    export_format: &str,
    rows: Vec<UsageRow>,
    now: DateTime<Utc>,
    transform: fn(&[UsageRow]) -> Result<FocusTable, TransformError>,
) -> Harness {
    let destination = RecordingDestination::default();
    let source = RecordingSource {
        rows,
        ..RecordingSource::default()
    };
    let engine = FocusExportEngine {
        format: format_for(export_format).expect("the test names a supported format"),
        destination: destination.clone(),
        source: source.clone(),
        clock: FixedClock { now },
        transform,
    };
    Harness {
        engine,
        destination,
        source,
    }
}
