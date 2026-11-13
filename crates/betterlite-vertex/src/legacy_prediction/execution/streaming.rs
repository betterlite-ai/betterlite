use std::future::Future;

use betterlite_models::decisions::Json;
use futures_util::StreamExt;

use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

use super::super::client::{
    EndpointPredictRequest, PredictionClient, PredictionStream, PublisherPredictRequest,
};
use super::super::error::{LegacyPredictionError, PredictionError};
use super::super::invocation::{LegacyInvocation, remove_instance_stream};
use super::super::records::{LegacyChunk, Prediction, PredictionsReply, strip_output_separator};
use super::super::streamer::TextStreamer;
use super::scope::{RpcScope, require_custom, resolve_scope};
use super::{LegacyChunkStream, LegacyContext, StreamFailure};

pub(super) async fn stream_inner<C, Lookup>(
    client: &C,
    invocation: LegacyInvocation,
    context: &LegacyContext<'_, '_>,
    lookup: Lookup,
) -> Result<LegacyChunkStream, StreamFailure>
where
    C: PredictionClient,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    prepare_stream(&invocation, context)?;
    let scope = resolve_scope(context, lookup).await?;
    let stream = stream_modes(client, context, scope, invocation).await?;
    Ok(stream
        .map(|item| item.map_err(|source| LegacyPredictionError::Streaming { source }))
        .boxed())
}

fn prepare_stream(
    invocation: &LegacyInvocation,
    context: &LegacyContext<'_, '_>,
) -> Result<(), StreamFailure> {
    if matches!(invocation, LegacyInvocation::Vision { .. }) {
        return Err(StreamFailure::Unable);
    }
    if matches!(invocation, LegacyInvocation::CustomEndpoint { .. }) {
        require_custom(context)?;
    }
    if let LegacyInvocation::PrivateEndpoint { instances, .. } = invocation
        && instances.is_empty()
    {
        return Err(StreamFailure::Prediction(PredictionError::MissingInstances));
    }
    Ok(())
}

async fn stream_modes<C>(
    client: &C,
    context: &LegacyContext<'_, '_>,
    scope: RpcScope,
    invocation: LegacyInvocation,
) -> Result<PredictionStream, PredictionError>
where
    C: PredictionClient,
{
    match invocation {
        LegacyInvocation::Chat { .. } | LegacyInvocation::Text { .. } => {
            publisher_stream(client, scope, invocation).await
        }
        LegacyInvocation::CustomEndpoint { .. } | LegacyInvocation::PrivateEndpoint { .. } => {
            endpoint_stream(client, scope, invocation).await
        }
        LegacyInvocation::Vision { .. } => Err(PredictionError::MissingPrediction),
    }
}

async fn publisher_stream<C>(
    client: &C,
    scope: RpcScope,
    invocation: LegacyInvocation,
) -> Result<PredictionStream, PredictionError>
where
    C: PredictionClient,
{
    match invocation {
        LegacyInvocation::Chat {
            model,
            prompt,
            mut options,
        } => {
            options.shift_remove("stream");
            let request = PublisherPredictRequest {
                token: scope.token,
                project: scope.project,
                location: scope.location,
                model,
                prompt,
                options,
            };
            client.send_message_streaming_async(&request).await
        }
        LegacyInvocation::Text {
            model,
            prompt,
            mut options,
        } => {
            options.shift_remove("stream");
            let request = PublisherPredictRequest {
                token: scope.token,
                project: scope.project,
                location: scope.location,
                model,
                prompt,
                options,
            };
            client.predict_streaming_async(&request).await
        }
        LegacyInvocation::CustomEndpoint { .. }
        | LegacyInvocation::PrivateEndpoint { .. }
        | LegacyInvocation::Vision { .. } => Err(PredictionError::MissingPrediction),
    }
}

async fn endpoint_stream<C>(
    client: &C,
    scope: RpcScope,
    invocation: LegacyInvocation,
) -> Result<PredictionStream, PredictionError>
where
    C: PredictionClient,
{
    match invocation {
        LegacyInvocation::CustomEndpoint {
            model,
            instances,
            mut options,
            ..
        } => {
            options.shift_remove("stream");
            let endpoint = format!(
                "projects/{}/locations/{}/endpoints/{}",
                scope.project, scope.location, model
            );
            let request = EndpointPredictRequest {
                token: scope.token,
                location: scope.location,
                endpoint,
                instances,
            };
            text_chunks(client, &request).await
        }
        LegacyInvocation::PrivateEndpoint {
            endpoint,
            mut instances,
            mut options,
            ..
        } => {
            options.shift_remove("stream");
            remove_instance_stream(&mut instances);
            let request = EndpointPredictRequest {
                token: scope.token,
                location: scope.location,
                endpoint,
                instances,
            };
            text_chunks(client, &request).await
        }
        LegacyInvocation::Chat { .. }
        | LegacyInvocation::Text { .. }
        | LegacyInvocation::Vision { .. } => Err(PredictionError::MissingPrediction),
    }
}

async fn text_chunks<C>(
    client: &C,
    request: &EndpointPredictRequest,
) -> Result<PredictionStream, PredictionError>
where
    C: PredictionClient,
{
    let reply = client.predict_endpoint(request).await?;
    let text = streaming_text(&reply)?;
    Ok(StreamExt::map(TextStreamer::new(&text), |word| {
        Ok(LegacyChunk { text: word })
    })
    .boxed())
}

fn streaming_text(reply: &PredictionsReply) -> Result<String, PredictionError> {
    let prediction = reply
        .predictions
        .first()
        .ok_or(PredictionError::MissingPrediction)?;
    let stripped = strip_output_separator(prediction);
    let Prediction::Value(Json::Text(text)) = stripped else {
        return Err(PredictionError::WrongFieldType {
            field: "predictions",
        });
    };
    Ok(text)
}
