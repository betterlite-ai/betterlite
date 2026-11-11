use serde::Deserialize;

use crate::catalog::ModelCatalog;
use crate::metadata::MetadataFields;

use super::resolve::split_model;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ModelLimits {
    pub max_tokens: Option<u64>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

impl ModelLimits {
    #[must_use]
    pub const fn from_fields(fields: &MetadataFields) -> Self {
        Self {
            max_tokens: fields.max_tokens,
            max_input_tokens: fields.max_input_tokens,
            max_output_tokens: fields.max_output_tokens,
        }
    }

    // Limits the entry leaves absent take the fill value, as the fill-missing backfill does.
    #[must_use]
    pub fn or(self, fill: Self) -> Self {
        Self {
            max_tokens: self.max_tokens.or(fill.max_tokens),
            max_input_tokens: self.max_input_tokens.or(fill.max_input_tokens),
            max_output_tokens: self.max_output_tokens.or(fill.max_output_tokens),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MaxTokensError {
    #[error(
        "Model {model} isn't mapped yet. Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json"
    )]
    Unmapped { model: String },
}

fn exact_limits(catalog: &ModelCatalog, name: &str) -> Option<ModelLimits> {
    let canonical = catalog.lookup.get(name)?;
    let entry = catalog.entries.get(canonical.as_ref())?;
    Some(ModelLimits::from_fields(&entry.fields.metadata))
}

// Exact catalog keys only: the model as given, then the one after the provider prefix; huggingface answers None, where LiteLLM fetches max_position_embeddings.
pub fn max_output_tokens(
    catalog: &ModelCatalog,
    model: &str,
) -> Result<Option<u64>, MaxTokensError> {
    let as_given = exact_limits(catalog, model)
        .and_then(|limits| limits.max_output_tokens.or(limits.max_tokens));
    if as_given.is_some() {
        return Ok(as_given);
    }
    let split = split_model(catalog, model);
    if split.provider.as_deref() == Some("huggingface") {
        return Ok(None);
    }
    match exact_limits(catalog, &split.model) {
        Some(limits) => Ok(limits.max_output_tokens.or(limits.max_tokens)),
        None => Err(MaxTokensError::Unmapped { model: split.model }),
    }
}
