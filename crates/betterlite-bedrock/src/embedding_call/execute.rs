use std::time::Duration;

use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::execute::execute as cohere_execute;
use betterlite_openai::cohere_legacy_embedding::execute::{ExecuteInput, LegacyEmbeddingTransport};
use betterlite_openai::cohere_legacy_embedding::plan::{PlanInput, plan as cohere_plan};
use betterlite_openai::cohere_legacy_embedding::{SYNC_CLIENT_TIMEOUT, SdkMode};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::EmbeddingCallError;
use super::hooks::{Event, Input, Recorder, replay};
use super::plan::{EmbeddingPlan, remaining_options};
use super::sign::{AsyncSigningInput, SigningInput, sign_item, sign_owned};
use super::{CredentialSource, Dependencies, EmbeddingCall, EmbeddingCallResponse, LogContext};
use crate::bedrock_embedding_io::response::{EmbeddingReply, transform_response};
use crate::bedrock_embedding_io::transport::{
    ClientFactory, ClientKind, EmbeddingCall as IoCall, SuppliedClient, Timeout, post_embedding,
};
use crate::signing::executor::AwsSigningExecutor;
use crate::signing::request::AwsSignedRequest;

struct Item<'call, 'text, 'plan> {
    call: &'call EmbeddingCall<'text>,
    plan: &'plan EmbeddingPlan,
}

struct BatchClients<'client, Factory: ClientFactory> {
    supplied: Option<(ClientKind, &'client Factory::Client)>,
    factory: &'client Factory,
    signing: &'client AwsSigningExecutor,
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
        let signed = sign(item, serde_json::to_vec(data)?, clients.signing).await?;
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

async fn sign(
    item: &Item<'_, '_, '_>,
    body: Vec<u8>,
    signing: &AwsSigningExecutor,
) -> Result<AwsSignedRequest, EmbeddingCallError> {
    if !item.call.aembedding {
        return Ok(sign_item(&body, signing_input(item))?);
    }
    let input = async_input(item, body);
    Ok(signing.run(move || sign_owned(input)).await??)
}

fn async_input(item: &Item<'_, '_, '_>, body: Vec<u8>) -> AsyncSigningInput {
    AsyncSigningInput {
        endpoint_url: item.plan.endpoint_url.clone(),
        body,
        credentials: item.plan.credentials.clone(),
        region: item.plan.region.clone(),
        extra_headers: item.call.extra_headers.unwrap_or_default().to_vec(),
        api_key: item.call.api_key.cloned(),
        configured_bearer: item.call.environment.bearer_token.cloned(),
        now: item.call.now,
    }
}

fn signing_input(item: &Item<'_, '_, '_>) -> SigningInput<'_> {
    SigningInput {
        endpoint_url: &item.plan.endpoint_url,
        credentials: item.plan.credentials.as_ref(),
        region: &item.plan.region,
        extra_headers: item.call.extra_headers.unwrap_or_default(),
        api_key: item.call.api_key,
        configured_bearer: item.call.environment.bearer_token,
        now: item.call.now,
    }
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
    let signed = sign_item(&body, signing_input(&Item { call, plan }))?;
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
    let mut recorder = Recorder::default();
    let input = ExecuteInput {
        plan: &legacy,
        input: call.input,
        api_key: None,
        encoding: dependencies.encoding,
    };
    // only a supplied handler of the branch's own kind replaces the default client, as in the handler
    let outcome = match supplied {
        Some(client)
            if (call.aembedding && client.kind == ClientKind::Async)
                || (!call.aembedding && client.kind == ClientKind::Sync) =>
        {
            cohere_execute(client.transport, input, &mut recorder).await
        }
        _ => cohere_execute(dependencies.transport, input, &mut recorder).await,
    };
    replay(recorder, log)?;
    Ok(outcome?)
}

// B1418's plan keeps one deadline: the blocking handler ignores the caller timeout and the async default is 600 seconds too
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
