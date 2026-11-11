use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::records::Chunk;
use super::response::BaseResponse;

#[derive(Debug, Error)]
#[error("{reason}")]
pub struct ProjectionError {
    pub reason: String,
}

#[derive(Debug, Error)]
enum ProviderMetadataError {
    #[error("resolving the provider for {model} failed: {source}")]
    Resolution {
        model: String,
        #[source]
        source: ResolutionError,
    },
    #[error("{provider} is not a known provider")]
    UnknownProvider { provider: String },
    #[error("selecting the chat config for {model} failed: {source}")]
    Selection {
        model: String,
        #[source]
        source: ChatConfigError,
    },
    #[error("the provider metadata projection failed: {source}")]
    Projection {
        #[source]
        source: ProjectionError,
    },
}

// B174: the provider a bare model string resolves to.
pub trait ProviderResolution: Send + Sync {
    fn provider_for_model(&self, model: &str) -> Result<ProviderId, ResolutionError>;
}

// B2230: the chat config tag the selected provider and model pair names.
pub trait ChatConfigSelection: Send + Sync {
    fn select_chat_config(
        &self,
        model: &str,
        provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError>;
}

// The hook a provider implementation installs to merge its own chunk metadata into the response.
pub trait ProviderMetadataProjection: Send + Sync {
    fn project(
        &self,
        tag: ChatConfigTag,
        response: &mut BaseResponse,
        chunks: &[Chunk],
    ) -> Result<(), ProjectionError>;
}

pub struct ProviderMetadataContext<'context> {
    pub custom_llm_provider: Option<&'context str>,
    pub redactor: &'context Redactor,
    pub resolver: &'context dyn ProviderResolution,
    pub selector: &'context dyn ChatConfigSelection,
    pub projection: Option<&'context dyn ProviderMetadataProjection>,
}

pub fn apply_provider_metadata(
    response: &mut BaseResponse,
    chunks: &[Chunk],
    context: &ProviderMetadataContext<'_>,
) {
    if chunks.is_empty() {
        return;
    }
    let model = response.model.clone();
    if model.is_empty() {
        return;
    }
    if let Err(error) = project_for_model(response, chunks, &model, context) {
        // B187: the model and the failure text can carry a credential, so both go through the redactor
        tracing::debug!(
            "apply_provider_assembled_streaming_metadata failed for model={}: {}",
            context.redactor.redact_string(&model),
            context.redactor.redact_string(&error.to_string()),
        );
    }
}

// The debug-and-return boundary: every resolution, selection and projection failure ends as one log line.
fn project_for_model(
    response: &mut BaseResponse,
    chunks: &[Chunk],
    model: &str,
    context: &ProviderMetadataContext<'_>,
) -> Result<(), ProviderMetadataError> {
    let provider = match context
        .custom_llm_provider
        .filter(|provider| !provider.is_empty())
    {
        Some(provider) => provider.parse::<ProviderId>().map_err(|_error| {
            ProviderMetadataError::UnknownProvider {
                provider: provider.to_owned(),
            }
        })?,
        None => context
            .resolver
            .provider_for_model(model)
            .map_err(|source| ProviderMetadataError::Resolution {
                model: model.to_owned(),
                source,
            })?,
    };
    let Some(tag) = context
        .selector
        .select_chat_config(model, provider)
        .map_err(|source| ProviderMetadataError::Selection {
            model: model.to_owned(),
            source,
        })?
    else {
        return Ok(());
    };
    if let Some(projection) = context.projection {
        projection
            .project(tag, response, chunks)
            .map_err(|source| ProviderMetadataError::Projection { source })?;
    }
    Ok(())
}
