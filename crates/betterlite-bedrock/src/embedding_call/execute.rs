use std::sync::Arc;
use std::time::Duration;

use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::execute::execute as cohere_execute;
use betterlite_openai::cohere_legacy_embedding::execute::{ExecuteInput, LegacyEmbeddingTransport};
use betterlite_openai::cohere_legacy_embedding::plan::{PlanInput, plan as cohere_plan};
use betterlite_openai::cohere_legacy_embedding::{SYNC_CLIENT_TIMEOUT, SdkMode};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};

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

pub async fn execute<Source, Factory, Transport>(
    call: &EmbeddingCall<'_>,
    plan: &EmbeddingPlan,
    dependencies: &Dependencies<'_, Source, Factory, Transport>,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    log: &mut LogContext<'_, '_>,
) -> Result<EmbeddingCallResponse, EmbeddingCallError>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
    Factory::Client: LegacyEmbeddingTransport,
    Transport: LegacyEmbeddingTransport,
{
    if let Some(batch) = plan.batch.as_deref() {
        let clients = BatchClients {
            supplied: supplied.map(|client| (client.kind, client.transport)),
            factory: dependencies.factory,
            signing: dependencies.signing,
            clock: &dependencies.clock,
        };
        let item = Item { call, plan };
        return Ok(EmbeddingCallResponse::Batch(
            run_batch(&item, batch, &clients, log).await?,
        ));
    }
    let Some(data) = plan.data.as_ref() else {
        return Err(EmbeddingCallError::Unmapped);
    };
    Ok(EmbeddingCallResponse::Legacy(
        cohere(call, plan, data, dependencies, supplied, log).await?,
    ))
}

async fn run_batch<Factory>(
    item: &Item<'_, '_, '_>,
    batch: &[Object],
    clients: &BatchClients<'_, Factory>,
    log: &mut LogContext<'_, '_>,
) -> Result<EmbeddingReply, EmbeddingCallError>
where
    Factory: ClientFactory + Sync,
{
    let mut responses = Vec::with_capacity(batch.len());
    for data in batch {
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
        responses.push(reply);
    }
    Ok(transform_response(
        &responses,
        &item.plan.model,
        item.plan.provider,
        item.plan.asynchronous,
        Some(batch),
    )?)
}

async fn cohere<Source, Factory, Transport>(
    call: &EmbeddingCall<'_>,
    plan: &EmbeddingPlan,
    data: &Object,
    dependencies: &Dependencies<'_, Source, Factory, Transport>,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    log: &mut LogContext<'_, '_>,
) -> Result<LegacyEmbeddingResponse, EmbeddingCallError>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
    Factory::Client: LegacyEmbeddingTransport,
    Transport: LegacyEmbeddingTransport,
{
    let body = serde_json::to_vec(data)?;
    let signed = sign_item(
        &body,
        SigningInput {
            endpoint_url: &plan.endpoint_url,
            credentials: plan.credentials.as_ref(),
            region: &plan.region,
            extra_headers: call.extra_headers.unwrap_or_default(),
            api_key: call.api_key,
            configured_bearer: call.environment.bearer_token,
            now: dependencies.clock.now(),
        },
    )?;
    let headers: Headers = signed.headers.iter().cloned().collect();
    let options = remaining_options(&call.optional_params)?;
    let legacy = cohere_plan(&PlanInput {
        mode: if call.aembedding {
            SdkMode::Async
        } else {
            SdkMode::Sync
        },
        model: &plan.model,
        input: call.input,
        optional_params: &options,
        supplied_data: Some(data),
        complete_api_base: Some(&signed.url),
        api_key: None,
        headers: &headers,
        timeout: cohere_timeout(call)?,
    });
    // LiteLLM logs the prepared request before the handler builds a client and sends it
    log_event(
        Event::Pre {
            input: Input::Texts(call.input.to_vec()),
            body: legacy.data.clone(),
            headers: call.aembedding.then(|| legacy.headers.clone()),
            api_base: call.aembedding.then(|| legacy.url.clone()),
        },
        log,
    )?;
    let sender = CohereSender {
        supplied,
        factory: dependencies.factory,
        fallback: dependencies.transport,
        asynchronous: call.aembedding,
        timeout: call.timeout.clone(),
    };
    let input = ExecuteInput {
        plan: &legacy,
        input: call.input,
        api_key: None,
        encoding: dependencies.encoding,
    };
    let mut recorder = Recorder::default();
    let outcome = cohere_execute(&sender, input, &mut recorder).await;
    replay(recorder, log)?;
    Ok(outcome?)
}

// B1418's plan keeps one deadline; the async branch also hands the four limits to the client factory, as LiteLLM does
fn cohere_timeout(call: &EmbeddingCall<'_>) -> Result<Duration, EmbeddingCallError> {
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
