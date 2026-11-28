use std::sync::Arc;

use betterlite_bedrock::bedrock_embedding_io::transport::ClientFactory as EmbeddingClientFactory;
use betterlite_bedrock::embedding_call::CredentialSource as EmbeddingCredentials;
use betterlite_bedrock::embedding_call::error::EmbeddingCallError;
use betterlite_bedrock::embedding_call::{
    Dependencies, EmbeddingCall, EmbeddingCallResponse, LogContext as EmbeddingLogContext,
    embeddings,
};
use betterlite_openai::cohere_legacy_embedding::execute::LegacyEmbeddingTransport;

use super::super::call::EmbeddingOperation;
use super::{LogContext, Services};

// LiteLLM's arm wraps a string input, then hands the leaf the same fields it was given
pub(super) async fn embedding<Source, Factory, Transport, Files>(
    operation: EmbeddingOperation<'_>,
    services: &Services<'_, Source, Factory, Transport, Files>,
    log: &mut LogContext<'_, '_>,
) -> Result<EmbeddingCallResponse, EmbeddingCallError>
where
    Source: EmbeddingCredentials + Sync,
    Factory: EmbeddingClientFactory + Sync,
    <Factory as EmbeddingClientFactory>::Client: LegacyEmbeddingTransport,
    Transport: LegacyEmbeddingTransport,
    Files: Sync,
{
    let input = operation.input.list();
    let mut call = EmbeddingCall {
        model: operation.model,
        input: input.as_ref(),
        optional_params: operation.optional_params,
        api_base: operation.api_base,
        extra_headers: operation.extra_headers,
        api_key: operation.api_key,
        timeout: operation.timeout,
        aembedding: operation.aembedding,
        drop_params: operation.drop_params,
        environment: operation.environment,
    };
    let dependencies = Dependencies {
        secrets: services.secrets,
        region_worker: services.region_worker,
        source: services.source,
        factory: services.factory,
        transport: services.transport,
        encoding: services.encoding,
        signing: services.signing,
        clock: Arc::clone(&services.clock),
    };
    let mut family_log = EmbeddingLogContext {
        state: &mut *log.state,
        env: log.env,
    };
    embeddings(&mut call, dependencies, None, &mut family_log).await
}
