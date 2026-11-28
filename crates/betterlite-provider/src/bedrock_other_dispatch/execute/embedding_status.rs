use betterlite_bedrock::bedrock_embedding_io::transport::ClientFactory as EmbeddingClientFactory;
use betterlite_bedrock::embedding_jobs::error::EmbeddingJobsError;
use betterlite_bedrock::embedding_jobs::logging::{StatusEvent, log_call};
use betterlite_bedrock::embedding_jobs::status::{
    AsyncInvokeStatusCall, StatusServices, get_async_invoke_status,
};
use betterlite_bedrock::embedding_jobs::transport::StatusTransport;
use betterlite_bedrock::image_call::plan::CredentialSource as ImageCredentials;
use betterlite_bedrock::image_call::plan::LogContext as ImageLogContext;
use betterlite_models::decisions::Json;

use super::super::call::EmbeddingStatusOperation;
use super::{LogContext, Services};

// B632's status lookup reads the job arn the caller already has; nothing here starts a job, reads S3 or polls
pub(super) async fn embedding_status<Source, Factory, Transport, Files>(
    operation: EmbeddingStatusOperation<'_>,
    services: &Services<'_, Source, Factory, Transport, Files>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, EmbeddingJobsError>
where
    Source: ImageCredentials + Sync,
    Factory: EmbeddingClientFactory + Sync,
    <Factory as EmbeddingClientFactory>::Client: StatusTransport,
    Transport: Sync,
    Files: Sync,
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
    // LiteLLM passes its logging object here, so the dispatcher's own context records both phases
    let mut family_log = ImageLogContext {
        state: &mut *log.state,
        env: log.env,
    };
    let mut logger = |invocation_arn: &str, event: StatusEvent<'_>| {
        log_call(invocation_arn, event, &mut family_log)
    };
    get_async_invoke_status(call, &status_services, Some(&mut logger)).await
}
