use std::str::FromStr;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::finops::focus::UsageData;
use rust_decimal::Decimal;

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("column '{column}' holds a value that is not a number")]
    NotNumeric { column: String },
    #[error("column '{column}' holds a number that is not a decimal")]
    NotDecimal {
        column: String,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("the sum of column '{column}' does not fit a decimal")]
    Overflow { column: String },
}

pub fn sum_column(rows: &UsageData, column: &str) -> Result<Decimal, SummaryError> {
    let mut total = Decimal::ZERO;
    for row in rows {
        let Some(value) = row.get(column) else {
            continue;
        };
        let addend = match value {
            JsonData::Null => Decimal::ZERO,
            JsonData::Number(number) => {
                Decimal::from_str(&number.to_string()).map_err(|source| {
                    SummaryError::NotDecimal {
                        column: column.to_owned(),
                        source,
                    }
                })?
            }
            JsonData::Bool(_) | JsonData::String(_) | JsonData::Array(_) | JsonData::Object(_) => {
                return Err(SummaryError::NotNumeric {
                    column: column.to_owned(),
                });
            }
        };
        total = total
            .checked_add(addend)
            .ok_or_else(|| SummaryError::Overflow {
                column: column.to_owned(),
            })?;
    }
    Ok(total)
}

#[must_use]
pub fn count_unique(rows: &UsageData, column: &str) -> usize {
    let mut seen: Vec<&JsonData> = Vec::new();
    for row in rows {
        let Some(value) = row.get(column) else {
            continue;
        };
        if !seen.contains(&value) {
            seen.push(value);
        }
    }
    seen.len()
}
