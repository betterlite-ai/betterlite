use std::sync::Arc;

use async_trait::async_trait;
use betterlite_core::operation::embedding::{
    EmbeddingCall, EmbeddingCallInput, EmbeddingFailure, EmbeddingResult,
};
use betterlite_openai::chat::deployment::Provider as DeploymentProvider;
use betterlite_openai::chat::execution::ChatExecutor;

use super::{EmbeddingCalls, embedding_mocks_resolve};

// a vertex_ai deployment reaches the installed Vertex call unless a configured mock answers first, as on every other provider
struct VertexFirst {
    vertex: Arc<dyn EmbeddingCall>,
    plain: EmbeddingCalls,
}

pub(super) fn calls(
    executor: Arc<ChatExecutor>,
    vertex: Option<Arc<dyn EmbeddingCall>>,
) -> Arc<dyn EmbeddingCall> {
    let plain = EmbeddingCalls { executor };
    match vertex {
        Some(installed) => Arc::new(VertexFirst {
            vertex: installed,
            plain,
        }),
        None => Arc::new(plain),
    }
}

#[async_trait]
impl EmbeddingCall for VertexFirst {
    async fn embed(
        &self,
        call: EmbeddingCallInput<'_>,
    ) -> Result<EmbeddingResult, EmbeddingFailure> {
        let executor = &self.plain.executor;
        if matches!(&call.deployment.provider, DeploymentProvider::Unsupported { model } if model.starts_with("vertex_ai/"))
            && !embedding_mocks_resolve(executor, &call.deployment, call.request)
                .map_err(EmbeddingFailure::Route)?
        {
            return self.vertex.embed(call).await;
        }
        self.plain.embed(call).await
    }
}
