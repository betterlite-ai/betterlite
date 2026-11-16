use std::error::Error;

use chrono::NaiveTime;

use crate::finops::focus::destinations::factory::DestinationProvider;
use crate::finops::focus::destinations::factory::error::FactoryError;
use crate::finops::focus::destinations::factory::resolve::DestinationOverrides;
use crate::finops::metering::clock::UtcClock;
use crate::focus_records::destinations::{FocusDestination, FocusTimeWindow};
use crate::focus_records::serializers::{FocusFormat, SerializeError};
use crate::focus_records::table::FocusTable;
use crate::focus_records::transformer::{TransformError, UsageRow, transform};

pub mod filename;
pub mod rows;
pub mod source;
pub mod summary;

mod tests;

use self::filename::FilenameError;
use self::source::UsageSource;

pub struct ExportSettings {
    pub provider: String,
    pub export_format: String,
    pub prefix: String,
    pub destination_config: DestinationOverrides,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError<DestinationError, SourceError>
where
    DestinationError: Error + Send + Sync + 'static,
    SourceError: Error + Send + Sync + 'static,
{
    #[error("the FOCUS destination could not be created")]
    Destination(#[from] FactoryError),
    #[error("Export format '{format}' not supported. Use 'parquet' or 'csv'.")]
    UnsupportedFormat { format: String },
    #[error("the FOCUS usage data could not be read")]
    Source(#[source] SourceError),
    #[error("the FOCUS usage data could not be normalized")]
    Transform(#[from] TransformError),
    #[error("the FOCUS export could not be serialized")]
    Serialize(#[from] SerializeError),
    #[error("the FOCUS export filename could not be built")]
    Filename(#[from] FilenameError),
    #[error("the FOCUS export could not be delivered")]
    Delivery(#[source] DestinationError),
}

pub struct FocusExportEngine<Destination, Source, Clock> {
    format: FocusFormat,
    destination: Destination,
    source: Source,
    clock: Clock,
    transform: fn(&[UsageRow]) -> Result<FocusTable, TransformError>,
}

impl<Destination, Source, Clock> FocusExportEngine<Destination, Source, Clock>
where
    Destination: FocusDestination + Sync,
    Destination::Error: Error + Send + Sync + 'static,
    Source: UsageSource,
    Source::Error: Error + Send + Sync + 'static,
    Clock: UtcClock,
{
    pub fn new<Factory>(
        settings: ExportSettings,
        factory: Factory,
        source: Source,
        clock: Clock,
    ) -> Result<Self, ExportError<Destination::Error, Source::Error>>
    where
        Factory: DestinationProvider<Destination = Destination>,
    {
        let destination = factory
            .create(
                &settings.provider,
                &settings.prefix,
                &settings.destination_config,
            )
            .map_err(ExportError::Destination)?;
        let Some(format) = format_for(&settings.export_format) else {
            return Err(ExportError::UnsupportedFormat {
                format: settings.export_format,
            });
        };
        Ok(Self {
            format,
            destination,
            source,
            clock,
            transform,
        })
    }

    pub async fn export_all(
        &self,
        limit: Option<i64>,
    ) -> Result<(), ExportError<Destination::Error, Source::Error>> {
        let data = self
            .source
            .usage_data(limit, None, None)
            .await
            .map_err(ExportError::Source)?;
        if data.is_empty() {
            tracing::debug!("Focus export: no usage data available");
            return Ok(());
        }
        let normalized = (self.transform)(&data)?;
        if normalized.rows == 0 {
            tracing::debug!("Focus export: normalized data empty");
            return Ok(());
        }
        let now = self.clock.now();
        let start_time = now.date_naive().and_time(NaiveTime::MIN).and_utc();
        let window = FocusTimeWindow {
            start_time,
            end_time: now,
            frequency: String::from("all"),
        };
        let payload = self.format.encode(&normalized)?;
        self.upload_payload(&payload, &window).await
    }

    pub async fn export_window(
        &self,
        window: &FocusTimeWindow,
        limit: Option<i64>,
    ) -> Result<(), ExportError<Destination::Error, Source::Error>> {
        let data = self
            .source
            .usage_data(limit, Some(window.start_time), Some(window.end_time))
            .await
            .map_err(ExportError::Source)?;
        if data.is_empty() {
            tracing::debug!("Focus export: no usage data for the window");
            return Ok(());
        }
        let normalized = (self.transform)(&data)?;
        if normalized.rows == 0 {
            tracing::debug!("Focus export: normalized data empty for the window");
            return Ok(());
        }
        let payload = self.format.encode(&normalized)?;
        self.upload_payload(&payload, window).await
    }

    async fn upload_payload(
        &self,
        payload: &[u8],
        window: &FocusTimeWindow,
    ) -> Result<(), ExportError<Destination::Error, Source::Error>> {
        if payload.is_empty() {
            tracing::debug!("Focus export: serializer returned empty payload");
            return Ok(());
        }
        let name = filename::build(window, self.format.extension())?;
        self.destination
            .deliver(payload, window, &name)
            .await
            .map_err(ExportError::Delivery)
    }
}

fn format_for(export_format: &str) -> Option<FocusFormat> {
    match export_format {
        "csv" => Some(FocusFormat::Csv),
        "parquet" => Some(FocusFormat::Parquet),
        _ => None,
    }
}
