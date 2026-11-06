use std::borrow::Cow;
use std::str::{Utf8Error, from_utf8};

pub const SUPPORTED_COLUMNS: [&str; 25] = [
    "ChargeCategory",
    "ChargePeriodStart",
    "BilledCost",
    "ServiceName",
    "BillingCurrency",
    "BillingAccountId",
    "BillingAccountName",
    "ChargePeriodEnd",
    "ChargeDescription",
    "ChargeFrequency",
    "ConsumedQuantity",
    "ConsumedUnit",
    "ContractedCost",
    "EffectiveCost",
    "ListCost",
    "RegionId",
    "RegionName",
    "ResourceId",
    "ResourceName",
    "ResourceType",
    "ServiceCategory",
    "ServiceSubcategory",
    "SubAccountId",
    "SubAccountName",
    "Tags",
];

#[derive(Debug, thiserror::Error)]
pub enum ColumnError {
    #[error("the CSV is not valid UTF-8")]
    NotUtf8(#[source] Utf8Error),
    #[error("the CSV could not be read or rewritten")]
    Csv(#[source] csv::Error),
    #[error("CSV record {record} has {found} fields but the header keeps column {needed}")]
    ShortRecord {
        record: usize,
        found: usize,
        needed: usize,
    },
}

// LiteLLM leaves the CR of a CRLF header on its last column name, which then never matches; we strip it.
pub fn strip_unsupported_columns(csv_bytes: &[u8]) -> Result<Cow<'_, [u8]>, ColumnError> {
    let first_line = csv_bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let header_bytes = first_line.strip_suffix(b"\r").unwrap_or(first_line);
    let header = from_utf8(header_bytes).map_err(ColumnError::NotUtf8)?;
    let names: Vec<&str> = header.split(',').collect();
    let supported = |name: &str| SUPPORTED_COLUMNS.contains(&name.trim_matches('"'));
    let keep: Vec<usize> = names
        .iter()
        .enumerate()
        .filter(|(_, name)| supported(name))
        .map(|(index, _)| index)
        .collect();
    if keep.len() == names.len() {
        return Ok(Cow::Borrowed(csv_bytes));
    }
    let dropped: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| !supported(name))
        .collect();
    tracing::debug!("Vantage destination: dropping unsupported columns: {dropped:?}");
    rewrite_with_columns(csv_bytes, &keep).map(Cow::Owned)
}

fn rewrite_with_columns(csv_bytes: &[u8], keep: &[usize]) -> Result<Vec<u8>, ColumnError> {
    let text = from_utf8(csv_bytes).map_err(ColumnError::NotUtf8)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    if keep.is_empty() {
        // The csv crate would write a field-less record as a quoted empty field; Python writes a bare line end.
        let mut line_ends = Vec::new();
        for parsed in reader.records() {
            parsed.map_err(ColumnError::Csv)?;
            line_ends.extend_from_slice(b"\r\n");
        }
        return Ok(line_ends);
    }
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .quote_style(csv::QuoteStyle::Necessary)
        .from_writer(Vec::new());
    for (position, parsed) in reader.records().enumerate() {
        let row = parsed.map_err(ColumnError::Csv)?;
        let mut fields = Vec::with_capacity(keep.len());
        for index in keep {
            let field = row.get(*index).ok_or_else(|| ColumnError::ShortRecord {
                record: position.saturating_add(1),
                found: row.len(),
                needed: *index,
            })?;
            fields.push(field);
        }
        writer.write_record(fields).map_err(ColumnError::Csv)?;
    }
    writer
        .into_inner()
        .map_err(|error| ColumnError::Csv(csv::Error::from(error.into_error())))
}
