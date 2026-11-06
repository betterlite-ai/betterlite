use std::time::Duration;

use chrono::NaiveDate;
use indexmap::IndexMap;
use jiff::tz::{TimeZone, TimeZoneDatabase};
use secrecy::{ExposeSecret as _, SecretString};
use tracing::warn;

use self::error::DeliveryError;
use self::payload::{BillingDrop, convert_record};
use self::timestamp::usage_start;
use self::transport::{BillingDropRequest, BillingTransport, Clock};
use super::records::CbfRecord;

pub mod error;
pub mod payload;
pub mod timestamp;
pub mod transport;

mod tests;

pub const DEFAULT_OPERATION: &str = "replace_hourly";
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const BASE_URL: &str = "https://api.cloudzero.com";

#[derive(Debug)]
pub struct Credentials {
    pub api_key: SecretString,
    pub connection_id: SecretString,
}

#[derive(Debug)]
pub struct CloudZeroStreamer<Sender, Time> {
    credentials: Credentials,
    user_timezone: TimeZone,
    clock: Time,
    transport: Sender,
}

impl<Sender: BillingTransport + Sync, Time: Clock> CloudZeroStreamer<Sender, Time> {
    pub fn new(
        credentials: Credentials,
        timezone_name: Option<&str>,
        database: &TimeZoneDatabase,
        clock: Time,
        transport: Sender,
    ) -> Self {
        let user_timezone =
            timezone_name
                .filter(|name| !name.is_empty())
                .map_or(TimeZone::UTC, |name| {
                    database.get(name).unwrap_or_else(|_| {
                        warn!(timezone = name, "Unknown timezone, using UTC");
                        TimeZone::UTC
                    })
                });
        Self {
            credentials,
            user_timezone,
            clock,
            transport,
        }
    }

    pub async fn send_batched(
        &self,
        data: &[CbfRecord],
        requested_operation: Option<&str>,
    ) -> Result<(), DeliveryError<Sender::Error>> {
        if data.is_empty() {
            return Ok(());
        }
        let operation = requested_operation.unwrap_or(DEFAULT_OPERATION);
        for (date, rows) in self.group_by_date(data) {
            self.send_daily_batch(date, &rows, operation).await?;
        }
        Ok(())
    }

    fn group_by_date<'rows>(
        &self,
        data: &'rows [CbfRecord],
    ) -> IndexMap<NaiveDate, Vec<&'rows CbfRecord>> {
        let mut daily_batches: IndexMap<NaiveDate, Vec<&CbfRecord>> = IndexMap::new();
        for row in data {
            match usage_start(row, &self.user_timezone) {
                Ok(Some(moment)) => {
                    daily_batches
                        .entry(moment.date_naive())
                        .or_default()
                        .push(row);
                }
                Ok(None) => {}
                Err(error) => warn!(%error, "skipping record with an unusable timestamp"),
            }
        }
        daily_batches
    }

    async fn send_daily_batch(
        &self,
        date: NaiveDate,
        rows: &[&CbfRecord],
        operation: &str,
    ) -> Result<(), DeliveryError<Sender::Error>> {
        let body = BillingDrop {
            month: date.format("%Y-%m").to_string(),
            operation,
            data: rows
                .iter()
                .map(|row| convert_record(row, &self.user_timezone, &self.clock))
                .collect(),
        };
        let request = BillingDropRequest {
            url: SecretString::from(format!(
                "{BASE_URL}/v2/connections/billing/anycost/{}/billing_drops",
                self.credentials.connection_id.expose_secret()
            )),
            authorization: SecretString::from(format!(
                "Bearer {}",
                self.credentials.api_key.expose_secret()
            )),
            timeout: REQUEST_TIMEOUT,
            body: &body,
        };
        self.transport
            .post(&request)
            .await
            .map_err(|source| DeliveryError::Send { date, source })
    }
}
