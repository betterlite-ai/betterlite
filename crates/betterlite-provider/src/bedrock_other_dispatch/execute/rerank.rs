use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::rerank::error::RerankCallError;
use betterlite_bedrock::rerank::execute::{dispatch_request, resolve_target, transform_response};
use betterlite_bedrock::rerank::plan::{
    LogContext as RerankLogContext, RerankCall, prepare_request,
};
use betterlite_bedrock::rerank::transport::ClientFactory as RerankClientFactory;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;

use super::super::call::RerankOperation;
use super::{LogContext, Services};

// The arm's four calls in order: resolve the agent-runtime target, sign and log the body, send it, decode it
pub(super) async fn rerank<Source, Factory, Transport, Files>(
    operation: RerankOperation<'_>,
    services: &Services<'_, Source, Factory, Transport, Files>,
    log: &mut LogContext<'_, '_>,
) -> Result<RerankResponse, RerankCallError>
where
    Source: CredentialSource,
    Factory: RerankClientFactory + Sync,
    Transport: Sync,
    Files: Sync,
{
    let mut call = RerankCall {
        request: &operation.request,
        optional_params: operation.optional_params,
        api_base: operation.api_base,
        extra_headers: operation.extra_headers,
        api_key: operation.api_key,
        timeout: operation.timeout,
        is_async: operation.is_async,
        environment: operation.environment,
        now: operation.now,
    };
    let target = resolve_target(&mut call, services.source).await?;
    let mut family_log = RerankLogContext {
        state: &mut *log.state,
        env: log.env,
    };
    let prepared = prepare_request(&call, &target, &mut family_log)?;
    let reply =
        dispatch_request(&call, &prepared, None, services.factory, &*family_log.state).await?;
    transform_response(&call, &reply, operation.generated_id, &mut family_log)
}
