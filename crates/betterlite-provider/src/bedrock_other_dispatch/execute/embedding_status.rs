use betterlite_bedrock::bedrock_embedding_io::transport::ClientFactory as EmbeddingClientFactory;
use betterlite_bedrock::embedding_jobs::error::EmbeddingJobsError;
use betterlite_bedrock::embedding_jobs::status::{
    AsyncInvokeStatusCall, StatusServices, get_async_invoke_status,
};
use betterlite_bedrock::embedding_jobs::transport::StatusTransport;
use betterlite_bedrock::image_call::plan::CredentialSource as ImageCredentials;
use betterlite_models::decisions::Json;

use super::Services;
use super::call::EmbeddingStatusOperation;

// B632's status lookup reads the job arn the caller already has; nothing here starts a job, reads S3 or polls
pub(super) async fn embedding_status<Source, Factory, Transport, Files>(
    operation: EmbeddingStatusOperation<'_>,
    services: &Services<'_, Source, Factory, Transport, Files>,
) -> Result<Json, EmbeddingJobsError>
where
    Source: ImageCredentials + Sync,
    Factory: EmbeddingClientFactory + Sync,
    <Factory as EmbeddingClientFactory>::Client: StatusTransport,
{
    let call = AsyncInvokeStatusCall {
        invocation_arn: operation.invocation_arn,
        region: operation.region,
        optional_params: operation.optional_params,
        environment: operation.environment,
        now: operation.now,
    };
    let status_services = StatusServices {
        source: services.source,
        secrets: services.secrets,
        region_worker: services.region_worker,
        factory: services.factory,
        signing: services.signing,
    };
    get_async_invoke_status(call, &status_services, None).await
}
