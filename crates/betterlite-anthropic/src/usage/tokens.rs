use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::shape::{UsageShapeError, declared_count, top_level_count};

#[derive(Clone, Copy)]
pub(super) struct TokenCounts {
    pub(super) prompt: i64,
    pub(super) completion: i64,
    pub(super) cache_creation: i64,
    pub(super) cache_read: i64,
}

impl TokenCounts {
    fn accumulate(&mut self, counts: Self) -> Result<(), UsageShapeError> {
        self.prompt = self
            .prompt
            .checked_add(counts.prompt)
            .ok_or(UsageShapeError::Overflow {
                field: "prompt_tokens",
            })?;
        self.completion =
            self.completion
                .checked_add(counts.completion)
                .ok_or(UsageShapeError::Overflow {
                    field: "completion_tokens",
                })?;
        self.cache_creation = self
            .cache_creation
            .checked_add(counts.cache_creation)
            .ok_or(UsageShapeError::Overflow {
                field: "cache_creation_input_tokens",
            })?;
        self.cache_read =
            self.cache_read
                .checked_add(counts.cache_read)
                .ok_or(UsageShapeError::Overflow {
                    field: "cache_read_input_tokens",
                })?;
        Ok(())
    }
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
    let base_prompt = top_level_count(fields.get("input_tokens"), "input_tokens")?;
    let completion = top_level_count(fields.get("output_tokens"), "output_tokens")?;
    let cache_creation = declared_count(
        fields.get("cache_creation_input_tokens"),
        "cache_creation_input_tokens",
    )?;
    let cache_read = declared_count(
        fields.get("cache_read_input_tokens"),
        "cache_read_input_tokens",
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
    let mut totals = TokenCounts {
        prompt: 0,
        completion: 0,
        cache_creation: 0,
        cache_read: 0,
    };
    for item in items {
        let Json::Object(fields) = item else {
            return Err(UsageShapeError::FieldType {
                field: "iterations",
            });
        };
        totals.accumulate(iteration_token_counts(fields)?)?;
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

fn iteration_token_counts(fields: &IndexMap<String, Json>) -> Result<TokenCounts, UsageShapeError> {
    Ok(TokenCounts {
        prompt: declared_count(fields.get("input_tokens"), "iterations.input_tokens")?,
        completion: declared_count(fields.get("output_tokens"), "iterations.output_tokens")?,
        cache_creation: declared_count(
            fields.get("cache_creation_input_tokens"),
            "iterations.cache_creation_input_tokens",
        )?,
        cache_read: declared_count(
            fields.get("cache_read_input_tokens"),
            "iterations.cache_read_input_tokens",
        )?,
    })
}
