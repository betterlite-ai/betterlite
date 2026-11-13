use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;

use super::token::{ChainError, ChainProviderFactory, ChainTokenProvider};

// B677 owns this chain but betterlite-azure depends on betterlite-openai, which depends on this crate (a cycle)
pub struct UnwiredChainFactory {
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
}

impl ChainProviderFactory for UnwiredChainFactory {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError> {
        let configured = [
            "AZURE_CLIENT_ID",
            "AZURE_TENANT_ID",
            "AZURE_FEDERATED_TOKEN_FILE",
        ]
        .iter()
        .filter(|name| {
            self.environment
                .lookup(name)
                .is_some_and(|value| !value.is_empty())
        })
        .count();
        if configured == 0 {
            return Err(ChainError::Unavailable);
        }
        Err(ChainError::Authentication {
            message: "the Azure credential chain is not wired into betterlite-logging".to_owned(),
        })
    }
}
