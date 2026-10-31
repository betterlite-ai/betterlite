use betterlite_config::rag::document::{
    INGESTION_PROVIDERS, IngestionProvider, ProviderIdentity as _,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IngestionProviderError {
    #[error(
        "Provider '{provider}' is not supported for RAG ingestion. Supported providers: {supported}"
    )]
    Unsupported { provider: String, supported: String },
}

pub fn get_ingestion_class(provider: &str) -> Result<IngestionProvider, IngestionProviderError> {
    if let Some(resolved) = IngestionProvider::from_wire_name(provider) {
        return Ok(resolved);
    }
    let supported = INGESTION_PROVIDERS
        .iter()
        .map(|known| known.wire_name())
        .collect::<Vec<_>>()
        .join(", ");
    Err(IngestionProviderError::Unsupported {
        provider: provider.to_owned(),
        supported,
    })
}
