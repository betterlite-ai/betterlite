use betterlite_context::anthropic_catalog_lookup::model_capability;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{CatalogSources, DropParams, unsupported_params};

pub const DROP_FORCED_TOOL_WARNING: &str = "Downgrading forced tool_choice to 'auto' for model=%s (drop_params=True): this model rejects tool_choice type 'any'/'tool' with a 400 because thinking is always on and a forced call would skip it.";

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum ForcedToolChoiceError {
    #[error(
        "{model} does not support forced tool use (tool_choice='required' or a named tool). Use tool_choice='auto' and tell the model in the prompt when to call the tool, or set `litellm.drop_params = True` to downgrade to 'auto' automatically."
    )]
    Unsupported { model: String },
    #[error("the tool_choice object has no type field")]
    MissingType,
    #[error("the tool_choice value is not an object")]
    NotAnObject,
}

impl ForcedToolChoiceError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Unsupported { .. } => Some(400),
            Self::MissingType | Self::NotAnObject => None,
        }
    }

    #[must_use]
    pub fn classify(&self) -> Option<ErrorClass> {
        match self {
            Self::Unsupported { .. } => Some(unsupported_params(self.to_string())),
            Self::MissingType | Self::NotAnObject => None,
        }
    }
}

#[must_use]
pub fn forced_tool_use_unsupported(model: &str, catalogs: &CatalogSources<'_>) -> bool {
    model_capability(
        catalogs.patterns,
        catalogs.runtime,
        catalogs.packaged,
        model,
        "supports_forced_tool_use",
    ) == Some(false)
}

pub fn forced_tool_use_downgraded(
    model: &str,
    drop: DropParams,
    catalogs: &CatalogSources<'_>,
) -> Result<bool, ForcedToolChoiceError> {
    if !forced_tool_use_unsupported(model, catalogs) {
        return Ok(false);
    }
    if !(drop.global || drop.call) {
        return Err(ForcedToolChoiceError::Unsupported {
            model: model.to_owned(),
        });
    }
    tracing::warn!("{}", DROP_FORCED_TOOL_WARNING.replacen("%s", model, 1));
    Ok(true)
}

pub fn apply_forced_tool_choice(
    model: &str,
    tool_choice: &Json,
    drop: DropParams,
    catalogs: &CatalogSources<'_>,
) -> Result<Json, ForcedToolChoiceError> {
    let Json::Object(fields) = tool_choice else {
        return Err(ForcedToolChoiceError::NotAnObject);
    };
    let Some(kind) = fields.get("type") else {
        return Err(ForcedToolChoiceError::MissingType);
    };
    let forced = match kind {
        Json::Text(text) => text == "any" || text == "tool",
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => false,
    };
    if !forced {
        return Ok(tool_choice.clone());
    }
    if !forced_tool_use_downgraded(model, drop, catalogs)? {
        return Ok(tool_choice.clone());
    }
    let mut downgraded = IndexMap::new();
    downgraded.insert("type".to_owned(), Json::Text("auto".to_owned()));
    if let Some(parallel) = fields.get("disable_parallel_tool_use")
        && !matches!(parallel, Json::Null)
    {
        downgraded.insert("disable_parallel_tool_use".to_owned(), parallel.clone());
    }
    Ok(Json::Object(downgraded))
}
