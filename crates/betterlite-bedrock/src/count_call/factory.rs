use super::counter::BedrockTokenCounter;
use super::handler::{
    BedrockCountTokensHandler, CountTokensCredentials, CountTokensEnvironment, CountTokensTransport,
};

#[derive(Debug, Clone, Default)]
pub struct BedrockModelInfo {
    pub environment: CountTokensEnvironment,
}

impl BedrockModelInfo {
    // LiteLLM's factory builds a counter from no state, so no credential lookup, gate or IO runs here
    #[must_use]
    pub fn get_token_counter<T, C>(
        &self,
        transport: T,
        credentials: C,
    ) -> BedrockTokenCounter<BedrockCountTokensHandler<T, C>>
    where
        T: CountTokensTransport,
        C: CountTokensCredentials,
    {
        BedrockTokenCounter {
            handler: BedrockCountTokensHandler {
                transport,
                credentials,
                environment: self.environment.clone(),
            },
        }
    }
}
