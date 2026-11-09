use std::error::Error as StdError;

use super::CacheCreationTokenDetails;
use super::count::{CountMode, count_value, optional_int};
use super::error::UsageError;
use super::wire::{CacheCreationValue, UsageIteration, UsageObject};

#[derive(Default)]
struct CreationTotals {
    five_minute: i64,
    one_hour: i64,
    total: i64,
    has_breakdown: bool,
}

impl CreationTotals {
    fn accumulate<CounterFailure>(
        &mut self,
        iteration: &UsageIteration,
    ) -> Result<(), UsageError<CounterFailure>>
    where
        CounterFailure: StdError + Send + Sync + 'static,
    {
        if let Some(CacheCreationValue::Mapping(creation)) = &iteration.cache_creation {
            self.has_breakdown = true;
            self.five_minute = self
                .five_minute
                .checked_add(count_value(
                    creation.ephemeral_5m_input_tokens.as_ref(),
                    "ephemeral_5m_input_tokens",
                    CountMode::Coerced,
                )?)
                .ok_or(UsageError::Overflow {
                    field: "cache_creation_token_details",
                })?;
            self.one_hour = self
                .one_hour
                .checked_add(count_value(
                    creation.ephemeral_1h_input_tokens.as_ref(),
                    "ephemeral_1h_input_tokens",
                    CountMode::Coerced,
                )?)
                .ok_or(UsageError::Overflow {
                    field: "cache_creation_token_details",
                })?;
        }
        self.total = self
            .total
            .checked_add(count_value(
                iteration.cache_creation_input_tokens.as_ref(),
                "cache_creation_input_tokens",
                CountMode::Coerced,
            )?)
            .ok_or(UsageError::Overflow {
                field: "cache_creation_token_details",
            })?;
        Ok(())
    }
}

pub(super) fn cache_creation_details<CounterFailure>(
    object: &UsageObject,
) -> Result<Option<CacheCreationTokenDetails>, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    if let Some(iterations) = object.present_iterations()
        && let Some(details) = aggregated_creation_details(iterations)?
    {
        return Ok(Some(details));
    }
    let Some(CacheCreationValue::Mapping(creation)) = &object.cache_creation else {
        return Ok(None);
    };
    Ok(Some(CacheCreationTokenDetails {
        ephemeral_5m: optional_int(
            creation.ephemeral_5m_input_tokens.as_ref(),
            "ephemeral_5m_input_tokens",
        )?,
        ephemeral_1h: optional_int(
            creation.ephemeral_1h_input_tokens.as_ref(),
            "ephemeral_1h_input_tokens",
        )?,
    }))
}

fn aggregated_creation_details<CounterFailure>(
    iterations: &[UsageIteration],
) -> Result<Option<CacheCreationTokenDetails>, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let mut totals = CreationTotals::default();
    for iteration in iterations {
        totals.accumulate(iteration)?;
    }
    if !totals.has_breakdown {
        return Ok(None);
    }
    let undetailed = undetailed_creation(totals.total, totals.five_minute, totals.one_hour)?;
    Ok(Some(CacheCreationTokenDetails {
        ephemeral_5m: Some(totals.five_minute.checked_add(undetailed).ok_or(
            UsageError::Overflow {
                field: "cache_creation_token_details",
            },
        )?),
        ephemeral_1h: Some(totals.one_hour),
    }))
}

fn undetailed_creation<CounterFailure>(
    total: i64,
    five_minute: i64,
    one_hour: i64,
) -> Result<i64, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    total
        .checked_sub(five_minute)
        .and_then(|value| value.checked_sub(one_hour))
        .map(|value| value.max(0))
        .ok_or(UsageError::Overflow {
            field: "cache_creation_token_details",
        })
}
