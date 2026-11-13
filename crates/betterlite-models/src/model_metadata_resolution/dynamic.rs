use std::convert::Infallible;
use std::error::Error;
use std::future::{Future, ready};

use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::identity::provider_id::ProviderId;
use crate::provider_metadata_factory::adapters::DynamicModelInfo;
use crate::provider_metadata_factory::selection::select;

// LiteLLM calls a provider config's get_model_info when it defines one; B2223 and B2224 answer for Ollama and Lemonade.
pub trait DynamicMetadataSource: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn get_model_info(
        &self,
        adapter: DynamicModelInfo,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> impl Future<Output = Result<Option<Box<RawValue>>, Self::Error>> + Send;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoDynamicMetadata;

impl DynamicMetadataSource for NoDynamicMetadata {
    type Error = Infallible;

    fn get_model_info(
        &self,
        _adapter: DynamicModelInfo,
        _model: &str,
        _api_base: Option<&str>,
        _api_key: Option<&SecretString>,
    ) -> impl Future<Output = Result<Option<Box<RawValue>>, Infallible>> + Send {
        ready(Ok(None))
    }
}

pub(super) async fn dynamic_model_info<Source>(
    source: &Source,
    model: &str,
    provider: Option<&str>,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> Option<Box<RawValue>>
where
    Source: DynamicMetadataSource,
{
    let selected = provider?;
    let adapter = select(Some(model), selected.parse::<ProviderId>().ok()?)?;
    let kind = adapter.dynamic_model_info()?;
    match source.get_model_info(kind, model, api_base, api_key).await {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                "Could not get dynamic model info for model={model}, provider={selected}; falling back to the static cost map: {error}"
            );
            None
        }
    }
}
