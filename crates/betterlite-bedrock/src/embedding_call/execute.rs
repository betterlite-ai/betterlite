use std::sync::Arc;
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_openai::cohere_legacy_embedding::SYNC_CLIENT_TIMEOUT;
use betterlite_openai::cohere_legacy_embedding::execute::LegacyEmbeddingTransport;
use betterlite_prompts::arguments::json::{Json, Object};
use futures_util::future::BoxFuture;

use super::error::EmbeddingCallError;
use super::hooks::{Event, Input, replay};
use super::plan::EmbeddingPlan;
use super::sign::{AsyncSigningInput, sign_owned};
use super::{Clock, Dependencies, EmbeddingCall, EmbeddingCallResponse, LogContext};
use crate::bedrock_embedding_io::response::{EmbeddingReply, transform_response};
use crate::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingCall as IoCall, Timeout, post_embedding,
};
use crate::credentials::CredentialSource;
use crate::signing::executor::AwsSigningExecutor;

mod bridge;

pub(super) struct Item<'call, 'text, 'plan> {
    call: &'call EmbeddingCall<'text>,
    plan: &'plan EmbeddingPlan,
}

struct BatchClients<'client, Factory: ClientFactory> {
    supplied: Option<(ClientKind, &'client Factory::Client)>,
    factory: &'client Factory,
    signing: &'client AwsSigningExecutor,
    clock: &'client Arc<dyn Clock>,
}

impl<Factory: ClientFactory + Sync> BatchClients<'_, Factory> {
    async fn deliver(&self, call: &IoCall<'_>) -> Result<Object, EmbeddingCallError> {
        let supplied = self
            .supplied
            .map(|(kind, transport)| SuppliedClient { kind, transport });
        Ok(post_embedding(call, supplied, self.factory).await?)
    }
}

#[must_use]
pub fn execute<'run, Source, Factory, Transport>(
    call: &'run EmbeddingCall<'_>,
    plan: &'run EmbeddingPlan,
    dependencies: &'run Dependencies<'_, Source, Factory, Transport>,
    supplied: Option<SuppliedClient<'run, Factory::Client>>,
    log: &'run mut LogContext<'_, '_>,
) -> BoxFuture<'run, Result<EmbeddingCallResponse, EmbeddingCallError>>
where
    Source: CredentialSource,
    Factory: ClientFactory + Sync,
    Factory::Client: LegacyEmbeddingTransport,
    Transport: LegacyEmbeddingTransport,
{
    Box::pin(async move {
        let item = Item { call, plan };
        if let Some(batch) = plan.batch.as_deref() {
            let clients = BatchClients {
                supplied: supplied.map(|client| (client.kind, client.transport)),
                factory: dependencies.factory,
                signing: dependencies.signing,
                clock: &dependencies.clock,
            };
            return Ok(EmbeddingCallResponse::Batch(
                run_batch(&item, batch, &clients, log).await?,
            ));
        }
        let Some(data) = plan.data.as_ref() else {
            return Err(EmbeddingCallError::Unmapped);
        };
        Ok(EmbeddingCallResponse::Legacy(
            bridge::cohere(&item, data, dependencies, supplied, log).await?,
        ))
    })
}

fn run_batch<'run, Factory>(
    item: &'run Item<'_, '_, '_>,
    batch: &'run [Object],
    clients: &'run BatchClients<'_, Factory>,
    log: &'run mut LogContext<'_, '_>,
) -> BoxFuture<'run, Result<EmbeddingReply, EmbeddingCallError>>
where
    Factory: ClientFactory + Sync,
{
    Box::pin(async move {
        let mut responses = Vec::with_capacity(batch.len());
        for data in batch {
            responses.push(item_call(item, data, clients, log).await?);
        }
        Ok(transform_response(
            &responses,
            &item.plan.model,
            item.plan.provider,
            item.plan.asynchronous,
            Some(batch),
        )?)
    })
}

fn item_call<'run, Factory>(
    item: &'run Item<'_, '_, '_>,
    data: &'run Object,
    clients: &'run BatchClients<'_, Factory>,
    log: &'run mut LogContext<'_, '_>,
) -> BoxFuture<'run, Result<Object, EmbeddingCallError>>
where
    Factory: ClientFactory + Sync,
{
    Box::pin(async move {
        let input = AsyncSigningInput {
            endpoint_url: item.plan.endpoint_url.clone(),
            body: serde_json::to_vec(data)?,
            credentials: item.plan.credentials.clone(),
            region: item.plan.region.clone(),
            extra_headers: item.call.extra_headers.unwrap_or_default().to_vec(),
            api_key: item.call.api_key.cloned(),
            configured_bearer: item.call.environment.bearer_token.cloned(),
            clock: Arc::clone(clients.clock),
        };
        let signed = clients.signing.run(move || sign_owned(input)).await??;
        replay(
            [Event::Pre {
                input: Input::Body,
                body: data.clone(),
                headers: Some(signed.headers.iter().cloned().collect()),
                api_base: Some(signed.url.clone()),
            }],
            log,
        )?;
        let reply = clients
            .deliver(&IoCall {
                asynchronous: item.call.aembedding,
                url: &item.plan.endpoint_url,
                headers: &signed.headers,
                data,
                timeout: item.call.timeout.clone(),
            })
            .await?;
        replay(
            [Event::Post {
                input: Input::Body,
                body: data.clone(),
                original_response: Json::Object(reply.clone()),
            }],
            log,
        )?;
        Ok(reply)
    })
}

// B1418's plan keeps one deadline; the async branch also hands the four limits to the client factory, as LiteLLM does
pub(super) fn cohere_timeout(call: &EmbeddingCall<'_>) -> Result<Duration, EmbeddingCallError> {
    if !call.aembedding {
        return Ok(SYNC_CLIENT_TIMEOUT);
    }
    let Some(timeout) = call.timeout.as_ref() else {
        return Ok(SYNC_CLIENT_TIMEOUT);
    };
    let seconds = match timeout {
        Timeout::Seconds(seconds) => *seconds,
        Timeout::Record(record) => record
            .connect
            .max(record.read)
            .max(record.write)
            .max(record.pool),
    };
    Duration::try_from_secs_f64(seconds).map_err(|_invalid| EmbeddingCallError::Timeout { seconds })
}
