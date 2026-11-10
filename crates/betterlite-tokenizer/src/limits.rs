use std::collections::BTreeSet;

use betterlite_models::catalog::{CatalogEntry, ModelCatalog};
use betterlite_models::identity::explicit_prefix::{
    PrefixClassification, PrefixClassificationInput, classify_prefix,
};
use rust_decimal::prelude::ToPrimitive as _;

mod tests;

pub const DEFAULT_BUFFER_FRACTION: f64 = 0.1;
pub const DEFAULT_BUFFER_MINIMUM_TOKENS: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub buffer_fraction: f64,
    pub buffer_minimum_tokens: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            buffer_fraction: DEFAULT_BUFFER_FRACTION,
            buffer_minimum_tokens: DEFAULT_BUFFER_MINIMUM_TOKENS,
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
    // LiteLLM's token_counter raised on missing messages, so the caller's limit stands
    InputNotCounted,
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
    input_tokens: Option<usize>,
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
        Err(warning) => {
            return Modified {
                max_tokens: Some(user),
                warnings: vec![warning],
            };
        }
    };
    let Some(counted) = input_tokens else {
        return Modified {
            max_tokens: Some(user),
            warnings: vec![Warning::InputNotCounted],
        };
    };
    let Some(buffer) = token_buffer(counted, settings) else {
        return Modified {
            max_tokens: Some(user),
            warnings: vec![Warning::BufferNotFinite],
        };
    };
    let Some(input) = counted.to_i128() else {
        return Modified {
            max_tokens: Some(user),
            warnings: vec![Warning::Overflow],
        };
    };
    adjusted(user, input, buffer, &limits)
}

fn model_limits(catalog: &ModelCatalog, models: Models<'_>) -> Result<ModelLimits, Warning> {
    let entry = catalog_entry(catalog, models.model).ok_or_else(|| Warning::UnknownModel {
        model: models.model.to_owned(),
    })?;
    let base = catalog_entry(catalog, models.base_model).ok_or_else(|| {
        Warning::UnknownMaxOutputTokens {
            base_model: models.base_model.to_owned(),
        }
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

// LiteLLM's get_max_tokens retries without the prefix only when get_llm_provider accepts it
fn catalog_entry<'catalog>(
    catalog: &'catalog ModelCatalog,
    name: &str,
) -> Option<&'catalog CatalogEntry> {
    lookup(catalog, name).or_else(|| {
        let configured_model_names = BTreeSet::new();
        let classification = classify_prefix(&PrefixClassificationInput {
            model: name,
            custom_llm_provider: None,
            configured_model_names: &configured_model_names,
            catalog,
        });
        let PrefixClassification::ExplicitProvider { model, .. } = classification else {
            return None;
        };
        lookup(catalog, &model)
    })
}

fn lookup<'catalog>(catalog: &'catalog ModelCatalog, name: &str) -> Option<&'catalog CatalogEntry> {
    let canonical = catalog.lookup.get(name)?;
    let entry = catalog.entries.get(canonical.as_ref())?;
    Some(entry)
}

fn adjusted(user: i64, input: i128, buffer: i128, limits: &ModelLimits) -> Modified {
    let limit = i128::from(limits.max_output_tokens);
    let buffered = input.saturating_add(buffer);
    let user_wide = i128::from(user);
    let shared = limits.max_input_tokens.map(i128::from) == Some(limit);
    if shared && buffered > limit {
        // LiteLLM lets the call fail normally rather than set a negative max_tokens
        return Modified {
            max_tokens: Some(user),
            warnings: Vec::new(),
        };
    }
    let room = if shared {
        let Some(room) = limit.checked_sub(buffered) else {
            return Modified {
                max_tokens: Some(user),
                warnings: vec![Warning::Overflow],
            };
        };
        room
    } else {
        limit
    };
    let Some(max_tokens) = i64::try_from(user_wide.min(room)).ok() else {
        return Modified {
            max_tokens: Some(user),
            warnings: vec![Warning::Overflow],
        };
    };
    Modified {
        max_tokens: Some(max_tokens),
        warnings: Vec::new(),
    }
}

fn token_buffer(input_tokens: usize, settings: &Settings) -> Option<i128> {
    let tokens = input_tokens.to_f64()?;
    let product = tokens.mul_add(settings.buffer_fraction, 0.0_f64);
    // Rust's f64::max ignores a NaN operand, so a NaN product stays NaN as Python's max keeps the first
    let buffer = if product.is_nan() {
        product
    } else {
        product.max(settings.buffer_minimum_tokens)
    };
    if !buffer.is_finite() {
        return None;
    }
    Some(buffer.trunc().to_i128().unwrap_or(i128::MAX))
}
