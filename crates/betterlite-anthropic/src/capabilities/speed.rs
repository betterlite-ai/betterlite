use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError;
use betterlite_context::anthropic_catalog_lookup::exact_capability;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::decisions::Json;

use crate::anthropic_capability_catalog::{CatalogSources, DropParams};

pub const DROP_UNSUPPORTED_SPEED_WARNING: &str = "Dropping unsupported `speed` for model=%s (drop_params=True). Fast mode is only supported on select Opus models.";

// Whether the caller sent "speed" at all; Present keeps a null value, which still raises or drops.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SpeedParam<'value> {
    #[default]
    Absent,
    Present(&'value Json),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeedDecision {
    Keep,
    Drop,
}

pub struct SpeedRequest<'request> {
    pub model: &'request str,
    pub speed: SpeedParam<'request>,
    pub drop: DropParams,
    pub custom_llm_provider: Option<&'request str>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpeedError {
    #[error(
        "{model} does not support speed={value}. To drop unsupported params, set `litellm.drop_params = True`."
    )]
    Unsupported { model: String, value: String },
    #[error("the unsupported speed value could not be rendered: {source}")]
    Value {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Catalog(#[from] CatalogLookupError),
}

impl SpeedError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Unsupported { .. } => Some(400),
            Self::Value { .. } | Self::Catalog(_) => None,
        }
    }

    #[must_use]
    pub fn classify(&self) -> Option<ErrorClass> {
        match self {
            Self::Unsupported { .. } => {
                Some(ErrorClass::UnsupportedParams(Box::new(ErrorContext {
                    message: self.to_string(),
                    model: None,
                    provider: None,
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                })))
            }
            Self::Value { .. } | Self::Catalog(_) => None,
        }
    }
}

// Fast mode is direct-Anthropic-only, and only the exact key in the runtime map may enable it.
pub fn model_supports_speed_param(
    model: &str,
    custom_llm_provider: Option<&str>,
    catalogs: &CatalogSources<'_>,
) -> Result<bool, SpeedError> {
    if custom_llm_provider.is_some_and(|provider| provider != "anthropic") {
        return Ok(false);
    }
    Ok(exact_capability(catalogs.runtime, model, "supports_speed")? == Some(true))
}

pub fn maybe_drop_speed_param(
    request: &SpeedRequest<'_>,
    catalogs: &CatalogSources<'_>,
) -> Result<SpeedDecision, SpeedError> {
    let SpeedParam::Present(value) = request.speed else {
        return Ok(SpeedDecision::Keep);
    };
    if model_supports_speed_param(request.model, request.custom_llm_provider, catalogs)? {
        return Ok(SpeedDecision::Keep);
    }
    if !(request.drop.global || request.drop.call) {
        return Err(SpeedError::Unsupported {
            model: request.model.to_owned(),
            value: serde_json::to_string(value).map_err(|source| SpeedError::Value { source })?,
        });
    }
    tracing::warn!(
        "{}",
        DROP_UNSUPPORTED_SPEED_WARNING.replacen("%s", request.model, 1)
    );
    Ok(SpeedDecision::Drop)
}
