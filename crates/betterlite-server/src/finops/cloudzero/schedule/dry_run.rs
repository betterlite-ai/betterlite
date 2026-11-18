use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Number;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::finops::cloudzero::{UsageRow, get_usage_data};
use betterlite_logging::finops::cloudzero::delivery::transport::{BillingTransport, Clock};
use betterlite_logging::finops::cloudzero::records::cbf::CbfTransformer;
use betterlite_logging::finops::cloudzero::records::{CbfRecord, CbfValue};

use super::CloudZeroLogger;
use super::error::CloudZeroExportError;
use super::usage_rows::{UsageRowError, decimal_field, integer_field, usage_records};

pub const DEFAULT_DRY_RUN_LIMIT: i64 = 10_000;
pub const SAMPLE_LIMIT: usize = 50;

pub type PreviewRecord = BTreeMap<String, JsonData>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRunSummary {
    pub total_records: usize,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_cost: Decimal,
    pub total_tokens: i64,
    pub unique_accounts: usize,
    pub unique_services: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRunExport {
    pub usage_data: Vec<UsageRow>,
    pub cbf_data: Vec<PreviewRecord>,
    pub summary: DryRunSummary,
}

#[derive(Debug, thiserror::Error)]
pub enum DryRunError {
    #[error("the dry run total for {field} overflowed")]
    TotalOverflow { field: &'static str },
    #[error("the dry run cost is not a JSON number")]
    Cost(#[source] serde_json::Error),
    #[error("the usage rows could not be read")]
    Usage(#[from] UsageRowError),
}

// dry_run_export_usage_data: a limit and nothing else, and no credential check before querying.
pub async fn dry_run_export_usage_data<Sender, Time>(
    logger: &CloudZeroLogger<Sender, Time>,
    limit: Option<i64>,
) -> Result<DryRunExport, CloudZeroExportError<Sender::Error>>
where
    Sender: BillingTransport,
    Time: Clock,
{
    let requested_limit = limit.unwrap_or(DEFAULT_DRY_RUN_LIMIT);
    let usage = get_usage_data(&logger.pipeline.store, Some(requested_limit), None, None).await?;
    if usage.is_empty() {
        tracing::warn!("CloudZero Dry Run: no usage data found");
        return Ok(DryRunExport {
            usage_data: Vec::new(),
            cbf_data: Vec::new(),
            summary: DryRunSummary {
                total_records: 0,
                total_cost: Decimal::ZERO,
                total_tokens: 0,
                unique_accounts: 0,
                unique_services: 0,
            },
        });
    }
    let usage_data: Vec<UsageRow> = usage.iter().take(SAMPLE_LIMIT).cloned().collect();
    let records = usage_records(&usage)?;
    let cbf_data = CbfTransformer::default().transform(&records);
    if cbf_data.is_empty() {
        tracing::warn!("CloudZero Dry Run: no valid data after transformation");
        let summary = sample_summary(&usage_data)?;
        return Ok(DryRunExport {
            usage_data,
            cbf_data: Vec::new(),
            summary,
        });
    }
    let preview = cbf_data
        .iter()
        .map(preview_record)
        .collect::<Result<Vec<PreviewRecord>, DryRunError>>()?;
    let summary = cbf_summary(&cbf_data)?;
    Ok(DryRunExport {
        usage_data,
        cbf_data: preview,
        summary,
    })
}

pub(crate) fn preview_record(record: &CbfRecord) -> Result<PreviewRecord, DryRunError> {
    let mut preview = PreviewRecord::new();
    for (field, value) in record {
        let preview_value = match value {
            CbfValue::None => JsonData::Null,
            CbfValue::Text(text) => JsonData::String(text.clone()),
            CbfValue::Integer(number) => JsonData::Number(Number::from(*number)),
            CbfValue::Money(amount) => JsonData::Number(
                serde_json::from_str::<Number>(&amount.to_string()).map_err(DryRunError::Cost)?,
            ),
            CbfValue::DateTime(moment) => {
                // datetime.isoformat(): no fraction at a whole second, six digits otherwise.
                let text = if moment.timestamp_subsec_micros() == 0 {
                    moment.naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string()
                } else {
                    moment
                        .naive_utc()
                        .format("%Y-%m-%dT%H:%M:%S%.6f")
                        .to_string()
                };
                JsonData::String(text)
            }
        };
        preview.insert(field.clone(), preview_value);
    }
    Ok(preview)
}

pub(crate) fn cbf_summary(records: &[CbfRecord]) -> Result<DryRunSummary, DryRunError> {
    let mut total_cost = Decimal::ZERO;
    let mut total_tokens = 0_i64;
    let mut accounts = BTreeSet::new();
    let mut services = BTreeSet::new();
    for record in records {
        if let Some(CbfValue::Money(cost)) = record.get("cost/cost") {
            total_cost = total_cost
                .checked_add(*cost)
                .ok_or(DryRunError::TotalOverflow {
                    field: "total_cost",
                })?;
        }
        if let Some(CbfValue::Integer(tokens)) = record.get("usage/amount") {
            total_tokens = total_tokens
                .checked_add(*tokens)
                .ok_or(DryRunError::TotalOverflow {
                    field: "total_tokens",
                })?;
        }
        if let Some(CbfValue::Text(account)) = record.get("resource/account") {
            if !account.is_empty() {
                accounts.insert(account.as_str());
            }
        }
        if let Some(CbfValue::Text(service)) = record.get("resource/service") {
            if !service.is_empty() {
                services.insert(service.as_str());
            }
        }
    }
    Ok(DryRunSummary {
        total_records: records.len(),
        total_cost,
        total_tokens,
        unique_accounts: accounts.len(),
        unique_services: services.len(),
    })
}

pub(crate) fn sample_summary(sample: &[UsageRow]) -> Result<DryRunSummary, DryRunError> {
    let mut total_cost = Decimal::ZERO;
    let mut total_tokens = 0_i64;
    for row in sample {
        let spend = decimal_field(row, "spend")?;
        total_cost = total_cost
            .checked_add(spend)
            .ok_or(DryRunError::TotalOverflow {
                field: "total_cost",
            })?;
        let row_tokens = integer_field(row, "prompt_tokens")?
            .checked_add(integer_field(row, "completion_tokens")?)
            .ok_or(DryRunError::TotalOverflow {
                field: "total_tokens",
            })?;
        total_tokens = total_tokens
            .checked_add(row_tokens)
            .ok_or(DryRunError::TotalOverflow {
                field: "total_tokens",
            })?;
    }
    Ok(DryRunSummary {
        total_records: sample.len(),
        total_cost,
        total_tokens,
        unique_accounts: 0,
        unique_services: 0,
    })
}
