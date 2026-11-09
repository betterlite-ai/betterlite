use betterlite_models::catalog::{CatalogEntry, ModelCatalog};
use rust_decimal::prelude::ToPrimitive as _;

mod tests;

pub const DEFAULT_BUFFER_PERC: f64 = 0.1;
pub const DEFAULT_BUFFER_NUM: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub buffer_perc: f64,
    pub buffer_num: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            buffer_perc: DEFAULT_BUFFER_PERC,
            buffer_num: DEFAULT_BUFFER_NUM,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Models<'names> {
    pub model: &'names str,
    pub base_model: &'names str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modified {
    pub max_tokens: Option<i64>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    // LiteLLM's get_model_info raised, so the caller's limit stands
    UnknownModel { model: String },
    // LiteLLM's get_max_tokens returned None or raised
    UnknownMaxOutputTokens { base_model: String },
    // Python's int() of a non-finite buffer raised, so the caller's limit stands
    BufferNotFinite,
    // betterlite's widened integer could not hold the value Python's int would
    Overflow,
}

struct ModelLimits {
    max_input_tokens: Option<u64>,
    max_output_tokens: u64,
}

#[must_use]
pub fn get_modified_max_tokens(
    catalog: &ModelCatalog,
    models: Models<'_>,
    user_max_tokens: Option<i64>,
    input_tokens: usize,
    settings: &Settings,
) -> Modified {
    let Some(user) = user_max_tokens else {
        return Modified {
            max_tokens: None,
            warnings: Vec::new(),
        };
    };
    let limits = match model_limits(catalog, models) {
        Ok(limits) => limits,
        Err(warning) => return fallback(user, warning),
    };
    let Some(buffer) = token_buffer(input_tokens, settings) else {
        return fallback(user, Warning::BufferNotFinite);
    };
    let Some(input) = input_tokens.to_i128() else {
        return fallback(user, Warning::Overflow);
    };
    adjusted(user, input, buffer, &limits)
}

fn model_limits(catalog: &ModelCatalog, models: Models<'_>) -> Result<ModelLimits, Warning> {
    let entry = lookup(catalog, models.model).ok_or_else(|| Warning::UnknownModel {
        model: models.model.to_owned(),
    })?;
    let base =
        lookup(catalog, models.base_model).ok_or_else(|| Warning::UnknownMaxOutputTokens {
            base_model: models.base_model.to_owned(),
        })?;
    let max_output_tokens = base
        .fields
        .metadata
        .max_output_tokens
        .or(base.fields.metadata.max_tokens)
        .ok_or_else(|| Warning::UnknownMaxOutputTokens {
            base_model: models.base_model.to_owned(),
        })?;
    Ok(ModelLimits {
        max_input_tokens: entry.fields.metadata.max_input_tokens,
        max_output_tokens,
    })
}

fn adjusted(user: i64, input: i128, buffer: i128, limits: &ModelLimits) -> Modified {
    let limit = i128::from(limits.max_output_tokens);
    let buffered = input.saturating_add(buffer);
    let user_wide = i128::from(user);
    let shared = limits.max_input_tokens.map(i128::from) == Some(limit);
    if shared && buffered > limit {
        // LiteLLM lets the call fail normally rather than set a negative max_tokens
        return resolved(user);
    }
    let over = if shared {
        user_wide.saturating_add(buffered) > limit
    } else {
        user_wide > limit
    };
    if !over {
        return resolved(user);
    }
    let value = if shared {
        let Some(value) = limit.checked_sub(buffered) else {
            return fallback(user, Warning::Overflow);
        };
        value
    } else {
        limit
    };
    let Some(max_tokens) = i64::try_from(value).ok() else {
        return fallback(user, Warning::Overflow);
    };
    resolved(max_tokens)
}

fn lookup<'catalog>(catalog: &'catalog ModelCatalog, name: &str) -> Option<&'catalog CatalogEntry> {
    match catalog.lookup.get(name) {
        Some(canonical) => catalog.entries.get(canonical.as_ref()),
        None => None,
    }
}

fn token_buffer(input_tokens: usize, settings: &Settings) -> Option<i128> {
    let tokens = input_tokens.to_f64()?;
    let product = tokens.mul_add(settings.buffer_perc, 0.0_f64);
    // Python's max returns the first argument whenever the second does not compare greater, NaN included
    let buffer = if settings.buffer_num > product {
        settings.buffer_num
    } else {
        product
    };
    if !buffer.is_finite() {
        return None;
    }
    Some(buffer.trunc().to_i128().unwrap_or(i128::MAX))
}

const fn resolved(max_tokens: i64) -> Modified {
    Modified {
        max_tokens: Some(max_tokens),
        warnings: Vec::new(),
    }
}

fn fallback(user_max_tokens: i64, warning: Warning) -> Modified {
    Modified {
        max_tokens: Some(user_max_tokens),
        warnings: vec![warning],
    }
}
