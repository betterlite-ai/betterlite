use std::future::Future;

use betterlite_models::decisions::Json;

use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

use super::super::client::{EndpointPredictRequest, PredictionClient, PublisherPredictRequest};
use super::super::error::PredictionError;
use super::super::invocation::LegacyInvocation;
use super::super::projection::{LegacyResult, ProjectionInput, project};
use super::super::records::{Prediction, ReplyMeta, strip_output_separator};
use super::LegacyContext;
use super::scope::{RpcScope, require_custom, resolve_scope};

pub(super) async fn complete_inner<C, Lookup>(
    client: &C,
    invocation: LegacyInvocation,
    context: &LegacyContext<'_, '_>,
    lookup: Lookup,
) -> Result<LegacyResult, PredictionError>
where
    C: PredictionClient,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    prepare_completion(&invocation, context)?;
    let scope = resolve_scope(context, lookup).await?;
    complete_modes(client, context, scope, invocation).await
}

async fn complete_modes<C>(
    client: &C,
    context: &LegacyContext<'_, '_>,
    scope: RpcScope,
    invocation: LegacyInvocation,
) -> Result<LegacyResult, PredictionError>
where
    C: PredictionClient,
{
    match invocation {
        LegacyInvocation::Chat { .. } | LegacyInvocation::Text { .. } => {
            publisher_completion(client, context, scope, invocation).await
        }
        LegacyInvocation::CustomEndpoint { .. } | LegacyInvocation::PrivateEndpoint { .. } => {
            endpoint_completion_call(client, context, scope, invocation).await
        }
        LegacyInvocation::Vision { model, prompt } => {
            projected(context, &model, &prompt, None, None)
        }
    }
}

async fn publisher_completion<C>(
    client: &C,
    context: &LegacyContext<'_, '_>,
    scope: RpcScope,
    invocation: LegacyInvocation,
) -> Result<LegacyResult, PredictionError>
where
    C: PredictionClient,
{
    match invocation {
        LegacyInvocation::Chat {
            model,
            prompt,
            options,
        } => {
            let request = PublisherPredictRequest {
                token: scope.token,
                project: scope.project,
                location: scope.location,
                model,
                prompt,
                options,
            };
            chat_completion(client, context, request).await
        }
        LegacyInvocation::Text {
            model,
            prompt,
            options,
        } => {
            let request = PublisherPredictRequest {
                token: scope.token,
                project: scope.project,
                location: scope.location,
                model,
                prompt,
                options,
            };
            text_completion(client, context, request).await
        }
        LegacyInvocation::CustomEndpoint { .. }
        | LegacyInvocation::PrivateEndpoint { .. }
        | LegacyInvocation::Vision { .. } => Err(PredictionError::CustomEndpointContext),
    }
}

async fn chat_completion<C>(
    client: &C,
    context: &LegacyContext<'_, '_>,
    request: PublisherPredictRequest,
) -> Result<LegacyResult, PredictionError>
where
    C: PredictionClient,
{
    let reply = client.send_message_async(&request).await?;
    let prediction = Prediction::Value(Json::Text(reply.text.clone()));
    projected(
        context,
        &request.model,
        &request.prompt,
        Some(&prediction),
        Some(&reply.meta),
    )
}

async fn text_completion<C>(
    client: &C,
    context: &LegacyContext<'_, '_>,
    request: PublisherPredictRequest,
) -> Result<LegacyResult, PredictionError>
where
    C: PredictionClient,
{
    let reply = client.predict_async(&request).await?;
    let prediction = Prediction::Value(Json::Text(reply.text.clone()));
    projected(
        context,
        &request.model,
        &request.prompt,
        Some(&prediction),
        Some(&reply.meta),
    )
}

async fn endpoint_completion_call<C>(
    client: &C,
    context: &LegacyContext<'_, '_>,
    scope: RpcScope,
    invocation: LegacyInvocation,
) -> Result<LegacyResult, PredictionError>
where
    C: PredictionClient,
{
    match invocation {
        LegacyInvocation::CustomEndpoint {
            model,
            prompt,
            instances,
            ..
        } => {
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
            endpoint_completion(client, context, request, &model, &prompt).await
        }
        LegacyInvocation::PrivateEndpoint {
            model,
            prompt,
            endpoint,
            instances,
            ..
        } => {
            let request = EndpointPredictRequest {
                token: scope.token,
                location: scope.location,
                endpoint,
                instances,
            };
            endpoint_completion(client, context, request, &model, &prompt).await
        }
        LegacyInvocation::Chat { .. }
        | LegacyInvocation::Text { .. }
        | LegacyInvocation::Vision { .. } => Err(PredictionError::MissingPrediction),
    }
}

async fn endpoint_completion<C>(
    client: &C,
    context: &LegacyContext<'_, '_>,
    request: EndpointPredictRequest,
    model: &str,
    prompt: &str,
) -> Result<LegacyResult, PredictionError>
where
    C: PredictionClient,
{
    let reply = client.predict_endpoint(&request).await?;
    let Some(prediction) = reply.predictions.first() else {
        return Err(PredictionError::MissingPrediction);
    };
    let stripped = strip_output_separator(prediction);
    projected(context, model, prompt, Some(&stripped), Some(&reply.meta))
}

fn projected(
    context: &LegacyContext<'_, '_>,
    model: &str,
    prompt: &str,
    prediction: Option<&Prediction>,
    meta: Option<&ReplyMeta>,
) -> Result<LegacyResult, PredictionError> {
    project(
        &ProjectionInput {
            prediction,
            meta,
            model_is_language: context.model_is_language,
            prompt,
            model,
        },
        context.services,
    )
}

fn prepare_completion(
    invocation: &LegacyInvocation,
    context: &LegacyContext<'_, '_>,
) -> Result<(), PredictionError> {
    if matches!(invocation, LegacyInvocation::CustomEndpoint { .. }) {
        require_custom(context)?;
    }
    Ok(())
}
