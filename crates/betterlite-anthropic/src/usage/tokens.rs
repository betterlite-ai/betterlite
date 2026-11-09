use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::shape::{CountMode, UsageShapeError, count_value};

#[derive(Clone, Copy, Default)]
pub(super) struct TokenCounts {
    pub(super) prompt: i64,
    pub(super) completion: i64,
    pub(super) cache_creation: i64,
    pub(super) cache_read: i64,
}

impl TokenCounts {
    fn add_fields(&mut self, fields: &IndexMap<String, Json>) -> Result<(), UsageShapeError> {
        add_count(
            &mut self.prompt,
            fields.get("input_tokens"),
            "iterations.input_tokens",
        )?;
        add_count(
            &mut self.completion,
            fields.get("output_tokens"),
            "iterations.output_tokens",
        )?;
        add_count(
            &mut self.cache_creation,
            fields.get("cache_creation_input_tokens"),
            "iterations.cache_creation_input_tokens",
        )?;
        add_count(
            &mut self.cache_read,
            fields.get("cache_read_input_tokens"),
            "iterations.cache_read_input_tokens",
        )?;
        Ok(())
    }
}

fn add_count(
    current: &mut i64,
    value: Option<&Json>,
    field: &'static str,
) -> Result<(), UsageShapeError> {
    *current = current
        .checked_add(count_value(value, field, CountMode::Declared)?)
        .ok_or(UsageShapeError::Overflow { field })?;
    Ok(())
}

pub(super) fn iteration_blocks(
    fields: &IndexMap<String, Json>,
) -> Result<Option<&[Json]>, UsageShapeError> {
    match fields.get("iterations") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(items)) => Ok((!items.is_empty()).then_some(items.as_slice())),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(UsageShapeError::FieldType {
                field: "iterations",
            })
        }
    }
}

pub(super) fn token_counts(
    fields: &IndexMap<String, Json>,
    iterations: Option<&[Json]>,
) -> Result<TokenCounts, UsageShapeError> {
    if let Some(items) = iterations {
        return iteration_counts(items);
    }
    top_level_counts(fields)
}

fn top_level_counts(fields: &IndexMap<String, Json>) -> Result<TokenCounts, UsageShapeError> {
    let base_prompt = count_value(
        fields.get("input_tokens"),
        "input_tokens",
        CountMode::Lenient,
    )?;
    let completion = count_value(
        fields.get("output_tokens"),
        "output_tokens",
        CountMode::Lenient,
    )?;
    let cache_creation = count_value(
        fields.get("cache_creation_input_tokens"),
        "cache_creation_input_tokens",
        CountMode::Declared,
    )?;
    let cache_read = count_value(
        fields.get("cache_read_input_tokens"),
        "cache_read_input_tokens",
        CountMode::Declared,
    )?;
    let prompt = base_prompt
        .checked_add(cache_creation)
        .and_then(|value| value.checked_add(cache_read))
        .ok_or(UsageShapeError::Overflow {
            field: "prompt_tokens",
        })?;
    Ok(TokenCounts {
        prompt,
        completion,
        cache_creation,
        cache_read,
    })
}

fn iteration_counts(items: &[Json]) -> Result<TokenCounts, UsageShapeError> {
    let mut totals = TokenCounts::default();
    for item in items {
        let Json::Object(fields) = item else {
            return Err(UsageShapeError::FieldType {
                field: "iterations",
            });
        };
        totals.add_fields(fields)?;
    }
    totals.prompt = totals
        .prompt
        .checked_add(totals.cache_creation)
        .and_then(|value| value.checked_add(totals.cache_read))
        .ok_or(UsageShapeError::Overflow {
            field: "prompt_tokens",
        })?;
    Ok(totals)
}
