use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::execute::execute as cohere_execute;
use betterlite_openai::cohere_legacy_embedding::execute::{ExecuteInput, LegacyEmbeddingTransport};
use betterlite_openai::cohere_legacy_embedding::plan::{
    LegacyEmbeddingPlan, PlanInput, plan as cohere_plan,
};
use betterlite_openai::cohere_legacy_embedding::{SYNC_CLIENT_TIMEOUT, SdkMode};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use futures_util::future::LocalBoxFuture;

use super::error::EmbeddingCallError;
use super::hooks::{Event, Input, Recorder, log_event, replay};
use super::plan::{EmbeddingPlan, remaining_options};
use super::sign::{AsyncSigningInput, SigningInput, sign_item, sign_owned};
use super::{
    Clock, CredentialSource, Dependencies, EmbeddingCall, EmbeddingCallResponse, LogContext,
};
use crate::bedrock_embedding_io::response::{EmbeddingReply, transform_response};
use crate::bedrock_embedding_io::transport::{
    ClientFactory, ClientKind, EmbeddingCall as IoCall, SuppliedClient, Timeout, post_embedding,
};
use crate::signing::executor::AwsSigningExecutor;
use crate::signing::request::AwsSignedRequest;
use bridge::CohereSender;

mod bridge;

struct Item<'call, 'text, 'plan> {
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
) -> LocalBoxFuture<'run, Result<EmbeddingCallResponse, EmbeddingCallError>>
where
    Source: CredentialSource + Sync,
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
            cohere(&item, data, dependencies, supplied, log).await?,
        ))
    })
}

fn run_batch<'run, Factory>(
    item: &'run Item<'_, '_, '_>,
    batch: &'run [Object],
    clients: &'run BatchClients<'_, Factory>,
    log: &'run mut LogContext<'_, '_>,
) -> LocalBoxFuture<'run, Result<EmbeddingReply, EmbeddingCallError>>
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
) -> LocalBoxFuture<'run, Result<Object, EmbeddingCallError>>
where
    Factory: ClientFactory + Sync,
{
    Box::pin(async move {
        let signed = sign_and_log(item, data, clients, log).await?;
        let call = IoCall {
            asynchronous: item.call.aembedding,
            url: &item.plan.endpoint_url,
            headers: &signed.headers,
            data,
            timeout: item.call.timeout.clone(),
        };
        let reply = clients.deliver(&call).await?;
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

fn sign_and_log<'run, Factory>(
    item: &'run Item<'_, '_, '_>,
    data: &'run Object,
    clients: &'run BatchClients<'_, Factory>,
    log: &'run mut LogContext<'_, '_>,
) -> LocalBoxFuture<'run, Result<AwsSignedRequest, EmbeddingCallError>>
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
        Ok(signed)
    })
}

fn cohere<'run, Source, Factory, Transport>(
    item: &'run Item<'_, '_, '_>,
    data: &'run Object,
    dependencies: &'run Dependencies<'_, Source, Factory, Transport>,
    supplied: Option<SuppliedClient<'run, Factory::Client>>,
    log: &'run mut LogContext<'_, '_>,
) -> LocalBoxFuture<'run, Result<LegacyEmbeddingResponse, EmbeddingCallError>>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
    Factory::Client: LegacyEmbeddingTransport,
    Transport: LegacyEmbeddingTransport,
{
    Box::pin(async move {
        let legacy = prepare(item, data, dependencies.clock.as_ref(), log)?;
        let sender = CohereSender {
            supplied,
            factory: dependencies.factory,
            fallback: dependencies.transport,
            asynchronous: item.call.aembedding,
            timeout: item.call.timeout.clone(),
            headers: &legacy.headers,
            timed_out: Mutex::new(None),
        };
        let input = ExecuteInput {
            plan: &legacy,
            input: item.call.input,
            api_key: None,
            encoding: dependencies.encoding,
        };
        let mut recorder = Recorder::default();
        let outcome = cohere_execute(&sender, input, &mut recorder).await;
        replay(recorder, log)?;
        if let Some(source) = sender.take_timeout() {
            return Err(EmbeddingCallError::CohereTimeout { source });
        }
        Ok(outcome?)
    })
}

// LiteLLM logs the prepared request before the handler builds a client and sends it
fn prepare(
    item: &Item<'_, '_, '_>,
    data: &Object,
    clock: &dyn Clock,
    log: &mut LogContext<'_, '_>,
) -> Result<LegacyEmbeddingPlan, EmbeddingCallError> {
    let signed = signed_body(item, data, clock)?;
    let legacy = legacy_plan(item, data, &signed)?;
    log_event(
        Event::Pre {
            input: Input::Texts(item.call.input.to_vec()),
            body: legacy.data.clone(),
            headers: item.call.aembedding.then(|| legacy.headers.clone()),
            api_base: item.call.aembedding.then(|| legacy.url.clone()),
        },
        log,
    )?;
    Ok(legacy)
}

fn signed_body(
    item: &Item<'_, '_, '_>,
    data: &Object,
    clock: &dyn Clock,
) -> Result<AwsSignedRequest, EmbeddingCallError> {
    let body = serde_json::to_vec(data)?;
    sign_item(
        &body,
        &SigningInput {
            endpoint_url: &item.plan.endpoint_url,
            credentials: item.plan.credentials.as_ref(),
            region: &item.plan.region,
            extra_headers: item.call.extra_headers.unwrap_or_default(),
            api_key: item.call.api_key,
            configured_bearer: item.call.environment.bearer_token,
            now: clock.now(),
        },
    )
    .map_err(EmbeddingCallError::from)
}

fn legacy_plan(
    item: &Item<'_, '_, '_>,
    data: &Object,
    signed: &AwsSignedRequest,
) -> Result<LegacyEmbeddingPlan, EmbeddingCallError> {
    let headers: Headers = signed.headers.iter().cloned().collect();
    let options = remaining_options(&item.call.optional_params)?;
    Ok(cohere_plan(&PlanInput {
        mode: if item.call.aembedding {
            SdkMode::Async
        } else {
            SdkMode::Sync
        },
        model: &item.plan.model,
        input: item.call.input,
        optional_params: &options,
        supplied_data: Some(data),
        complete_api_base: Some(&signed.url),
        api_key: None,
        headers: &headers,
        timeout: cohere_timeout(item.call)?,
    }))
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
