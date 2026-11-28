use std::sync::Arc;

use betterlite_bedrock::bedrock_embedding_io::credentials::ManagedRegionWorker;
use betterlite_bedrock::bedrock_embedding_io::transport::ClientFactory as EmbeddingClientFactory;
use betterlite_bedrock::count_call::handler::{
    CountTokensCredentials, CountTokensHandler, CountTokensTransport,
};
use betterlite_bedrock::embedding_call::Clock;
use betterlite_bedrock::embedding_call::CredentialSource as EmbeddingCredentials;
use betterlite_bedrock::embedding_jobs::transport::StatusTransport;
use betterlite_bedrock::image_call::plan::CredentialSource as ImageCredentials;
use betterlite_bedrock::image_call::transport::ClientFactory as ImageClientFactory;
use betterlite_bedrock::rerank::plan::CredentialSource as RerankCredentials;
use betterlite_bedrock::rerank::transport::ClientFactory as RerankClientFactory;
use betterlite_bedrock::signing::executor::AwsSigningExecutor;
use betterlite_bedrock::transcription::auth::ChainCredentials;
use betterlite_bedrock::transcription::binding::TranscriptionBinding;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::audio_file_input::AuthorizedAudioFiles;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::execute::LegacyEmbeddingTransport;
use betterlite_openai::transcription::transport::TranscriptionTransport;
use betterlite_secret_managers::secret_lookup::SecretLookup;

use crate::operation::{OperationRequest, OperationResult};

use self::count::count;
use self::embedding::embedding;
use self::embedding_status::embedding_status;
use self::image::image;
use self::rerank::rerank;
use self::transcription::transcription;
use super::call::BedrockOtherOperation;
use super::error::BedrockOtherError;
use super::{BedrockOtherRequest, BedrockOtherResult};

mod count;
mod embedding;
mod embedding_status;
mod image;
mod rerank;
mod transcription;

// The one dependency bundle every non-chat family reads; the family's own transport seam picks what it needs
pub struct Services<'deps, Source, Factory, Transport, Files> {
    pub secrets: &'deps SecretLookup,
    pub region_worker: &'deps ManagedRegionWorker,
    pub source: &'deps Source,
    pub factory: &'deps Factory,
    pub transport: &'deps Transport,
    pub encoding: &'deps (dyn TextEncoding + Send + Sync),
    pub signing: &'deps AwsSigningExecutor,
    pub clock: Arc<dyn Clock>,
    pub files: &'deps Files,
}

// The callback state every leaf logs through; each family keeps its own LogContext shape
pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

// The closed family enum is the only entry: a request for another arm is refused rather than guessed
pub async fn dispatch<Source, Factory, Transport, Files, H, B>(
    request: OperationRequest,
    operation: BedrockOtherOperation<'_, '_, '_, H, B>,
    services: &Services<'_, Source, Factory, Transport, Files>,
    log: &mut LogContext<'_, '_>,
) -> Result<OperationResult, BedrockOtherError<B::Error>>
where
    Source: EmbeddingCredentials
        + ImageCredentials
        + RerankCredentials
        + CountTokensCredentials
        + ChainCredentials
        + Sync,
    Factory: EmbeddingClientFactory + RerankClientFactory + ImageClientFactory + Sync,
    <Factory as EmbeddingClientFactory>::Client: LegacyEmbeddingTransport + StatusTransport,
    Transport: LegacyEmbeddingTransport + CountTokensTransport + TranscriptionTransport,
    Files: AuthorizedAudioFiles + Sync,
    H: CountTokensHandler,
    B: TranscriptionBinding + Send + Sync,
{
    let OperationRequest::BedrockOther(BedrockOtherRequest) = request else {
        return Err(BedrockOtherError::UnsupportedOperation);
    };
    let result = match operation {
        BedrockOtherOperation::Embedding(embedding_operation) => BedrockOtherResult::Embedding(
            Box::new(embedding(embedding_operation, services, log).await?),
        ),
        BedrockOtherOperation::EmbeddingStatus(status_operation) => {
            BedrockOtherResult::EmbeddingStatus(Box::new(
                embedding_status(status_operation, services, log).await?,
            ))
        }
        BedrockOtherOperation::Rerank(rerank_operation) => {
            BedrockOtherResult::Rerank(Box::new(rerank(rerank_operation, services, log).await?))
        }
        BedrockOtherOperation::Count(count_operation) => {
            BedrockOtherResult::Count(count(count_operation).await.map(Box::new))
        }
        BedrockOtherOperation::Transcription(transcription_operation) => {
            BedrockOtherResult::Transcription(Box::new(
                transcription(transcription_operation, services.files).await?,
            ))
        }
        BedrockOtherOperation::Image(image_operation) => {
            let dispatched = image(image_operation, services, log).await?;
            let OperationResult::BedrockImage(result) = dispatched else {
                return Err(BedrockOtherError::UnexpectedVariant);
            };
            BedrockOtherResult::Image(Box::new(result))
        }
        BedrockOtherOperation::Unsupported { operation: name } => {
            return Err(BedrockOtherError::Unsupported { operation: name });
        }
    };
    Ok(OperationResult::BedrockOther(result))
}
