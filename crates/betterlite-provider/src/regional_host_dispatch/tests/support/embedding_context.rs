use std::time::Duration;

use crate::regional_host_dispatch::context::EmbeddingContext;

use super::embedding::EmbeddingFixture;

impl EmbeddingFixture {
    pub(crate) fn context(&self) -> EmbeddingContext<'_> {
        EmbeddingContext {
            aembedding: false,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            extra_headers: self.extra_headers.as_ref(),
            input: &self.input,
            model: "embed-model",
            optional_params: &self.optional_params,
            ssl_verify: self.ssl_verify,
            timeout: Duration::from_secs(30),
            settings: &self.settings,
        }
    }
}
