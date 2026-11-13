use std::error::Error as StdError;

use super::count::{CountMode, count_value};
use super::error::UsageError;
use super::wire::{CountValue, UsageIteration, UsageObject};

#[derive(Clone, Copy, Default)]
pub(super) struct TokenCounts {
    pub(super) prompt: i64,
    pub(super) completion: i64,
    pub(super) cache_creation: i64,
    pub(super) cache_read: i64,
}

impl TokenCounts {
    fn add_iteration<CounterFailure>(
        &mut self,
        iteration: &UsageIteration,
    ) -> Result<(), UsageError<CounterFailure>>
    where
        CounterFailure: StdError + Send + Sync + 'static,
    {
        add_count(
            &mut self.prompt,
            iteration.input_tokens.as_ref(),
            "iterations.input_tokens",
        )?;
        add_count(
            &mut self.completion,
            iteration.output_tokens.as_ref(),
            "iterations.output_tokens",
        )?;
        add_count(
            &mut self.cache_creation,
            iteration.cache_creation_input_tokens.as_ref(),
            "iterations.cache_creation_input_tokens",
        )?;
        add_count(
            &mut self.cache_read,
            iteration.cache_read_input_tokens.as_ref(),
            "iterations.cache_read_input_tokens",
        )?;
        Ok(())
    }
}

fn add_count<CounterFailure>(
    current: &mut i64,
    value: Option<&CountValue>,
    field: &'static str,
) -> Result<(), UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    *current = current
        .checked_add(count_value(value, field, CountMode::Iteration)?)
        .ok_or(UsageError::Overflow { field })?;
    Ok(())
}

pub(super) fn token_counts<CounterFailure>(
    object: &UsageObject,
) -> Result<TokenCounts, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    if let Some(iterations) = object.present_iterations() {
        return iteration_counts(iterations);
    }
    top_level_counts(object)
}

fn top_level_counts<CounterFailure>(
    object: &UsageObject,
) -> Result<TokenCounts, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let base_prompt = count_value(
        object.input_tokens.as_ref(),
        "input_tokens",
        CountMode::Lenient,
    )?;
    let completion = count_value(
        object.output_tokens.as_ref(),
        "output_tokens",
        CountMode::Lenient,
    )?;
    let cache_creation = count_value(
        object.cache_creation_input_tokens.as_ref(),
        "cache_creation_input_tokens",
        CountMode::Declared,
    )?;
    let cache_read = count_value(
        object.cache_read_input_tokens.as_ref(),
        "cache_read_input_tokens",
        CountMode::Declared,
    )?;
    let prompt = base_prompt
        .checked_add(cache_creation)
        .and_then(|value| value.checked_add(cache_read))
        .ok_or(UsageError::Overflow {
            field: "prompt_tokens",
        })?;
    Ok(TokenCounts {
        prompt,
        completion,
        cache_creation,
        cache_read,
    })
}

fn iteration_counts<CounterFailure>(
    iterations: &[UsageIteration],
) -> Result<TokenCounts, UsageError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let mut totals = TokenCounts::default();
    for iteration in iterations {
        totals.add_iteration(iteration)?;
    }
    totals.prompt = totals
        .prompt
        .checked_add(totals.cache_creation)
        .and_then(|value| value.checked_add(totals.cache_read))
        .ok_or(UsageError::Overflow {
            field: "prompt_tokens",
        })?;
    Ok(totals)
}
