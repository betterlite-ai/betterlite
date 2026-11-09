use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::CacheCreationTokenDetails;
use super::shape::{UsageShapeError, coerced_count, optional_count};

#[derive(Default)]
struct CreationTotals {
    five_minute: i64,
    one_hour: i64,
    total: i64,
    has_breakdown: bool,
}

impl CreationTotals {
    fn accumulate(&mut self, fields: &IndexMap<String, Json>) -> Result<(), UsageShapeError> {
        if let Some(Json::Object(creation)) = fields.get("cache_creation") {
            self.has_breakdown = true;
            self.five_minute = self
                .five_minute
                .checked_add(coerced_count(
                    creation.get("ephemeral_5m_input_tokens"),
                    "ephemeral_5m_input_tokens",
                )?)
                .ok_or(UsageShapeError::Overflow {
                    field: "cache_creation_token_details",
                })?;
            self.one_hour = self
                .one_hour
                .checked_add(coerced_count(
                    creation.get("ephemeral_1h_input_tokens"),
                    "ephemeral_1h_input_tokens",
                )?)
                .ok_or(UsageShapeError::Overflow {
                    field: "cache_creation_token_details",
                })?;
        }
        self.total = self
            .total
            .checked_add(coerced_count(
                fields.get("cache_creation_input_tokens"),
                "cache_creation_input_tokens",
            )?)
            .ok_or(UsageShapeError::Overflow {
                field: "cache_creation_token_details",
            })?;
        Ok(())
    }
}

pub(super) fn cache_creation_details(
    fields: &IndexMap<String, Json>,
    iterations: Option<&[Json]>,
) -> Result<Option<CacheCreationTokenDetails>, UsageShapeError> {
    if let Some(items) = iterations
        && let Some(details) = aggregated_creation_details(items)?
    {
        return Ok(Some(details));
    }
    let Some(Json::Object(creation)) = fields.get("cache_creation") else {
        return Ok(None);
    };
    Ok(Some(CacheCreationTokenDetails {
        ephemeral_5m: optional_count(
            creation.get("ephemeral_5m_input_tokens"),
            "ephemeral_5m_input_tokens",
        )?,
        ephemeral_1h: optional_count(
            creation.get("ephemeral_1h_input_tokens"),
            "ephemeral_1h_input_tokens",
        )?,
    }))
}

fn aggregated_creation_details(
    items: &[Json],
) -> Result<Option<CacheCreationTokenDetails>, UsageShapeError> {
    let mut totals = CreationTotals::default();
    for item in items {
        let Json::Object(fields) = item else {
            return Err(UsageShapeError::FieldType {
                field: "iterations",
            });
        };
        totals.accumulate(fields)?;
    }
    if !totals.has_breakdown {
        return Ok(None);
    }
    let undetailed = undetailed_creation(totals.total, totals.five_minute, totals.one_hour)?;
    Ok(Some(CacheCreationTokenDetails {
        ephemeral_5m: Some(totals.five_minute.checked_add(undetailed).ok_or(
            UsageShapeError::Overflow {
                field: "cache_creation_token_details",
            },
        )?),
        ephemeral_1h: Some(totals.one_hour),
    }))
}

fn undetailed_creation(
    total: i64,
    five_minute: i64,
    one_hour: i64,
) -> Result<i64, UsageShapeError> {
    total
        .checked_sub(five_minute)
        .and_then(|value| value.checked_sub(one_hour))
        .map(|value| value.max(0))
        .ok_or(UsageShapeError::Overflow {
            field: "cache_creation_token_details",
        })
}
