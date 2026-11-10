use crate::catalog::ModelCatalog;
use crate::metadata::MetadataFields;

use super::info::{ModelInfoError, ModelQuery, find_entry};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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
}

#[derive(Debug, thiserror::Error)]
pub enum MaxTokensError {
    #[error(
        "Model {model} isn't mapped yet. Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json"
    )]
    Unmapped { model: String },
}

pub fn limits(
    catalog: &ModelCatalog,
    query: &ModelQuery<'_>,
) -> Result<ModelLimits, ModelInfoError> {
    find_entry(catalog, query)
        .map(|entry| ModelLimits::from_fields(&entry.fields.metadata))
        .ok_or_else(|| ModelInfoError::unmapped(query))
}

pub fn max_output_tokens(catalog: &ModelCatalog, model: &str) -> Result<u64, MaxTokensError> {
    let query = ModelQuery {
        model,
        provider: None,
    };
    let Some(entry) = find_entry(catalog, &query) else {
        return Err(MaxTokensError::Unmapped {
            model: model.to_owned(),
        });
    };
    entry
        .fields
        .metadata
        .max_output_tokens
        .or(entry.fields.metadata.max_tokens)
        .ok_or_else(|| MaxTokensError::Unmapped {
            model: model.to_owned(),
        })
}
