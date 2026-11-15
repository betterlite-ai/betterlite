use std::future::Future;

use betterlite_models::decisions::Json;

use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

use super::super::client::{EndpointPredictRequest, PredictionClient, PublisherPredictRequest};
use super::super::error::PredictionError;
use super::super::invocation::LegacyInvocation;
use super::super::projection::{LegacyResult, ProjectionInput, project};
use super::super::records::{Prediction, ReplyMeta, TextReply, strip_output_separator};
use super::LegacyContext;
use super::scope::{RpcScope, endpoint_path, require_custom, resolve_scope};

struct EndpointCall {
    model: String,
    prompt: String,
    endpoint: String,
    instances: Vec<Json>,
}

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
    match invocation {
        LegacyInvocation::Chat { .. } | LegacyInvocation::Text { .. } => {
            publisher_completion(client, context, scope, invocation).await
        }
        LegacyInvocation::CustomEndpoint { .. } | LegacyInvocation::PrivateEndpoint { .. } => {
            let Some(call) = endpoint_call(invocation, &scope) else {
                return Err(PredictionError::CustomEndpointContext);
            };
            endpoint_completion(client, context, scope, call).await
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
            let reply = client.send_message_async(&request).await?;
            text_result(context, &request, reply)
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
            let reply = client.predict_async(&request).await?;
            text_result(context, &request, reply)
        }
        LegacyInvocation::CustomEndpoint { .. }
        | LegacyInvocation::PrivateEndpoint { .. }
        | LegacyInvocation::Vision { .. } => Err(PredictionError::CustomEndpointContext),
    }
}

fn text_result(
    context: &LegacyContext<'_, '_>,
    request: &PublisherPredictRequest,
    reply: TextReply,
) -> Result<LegacyResult, PredictionError> {
    let prediction = Prediction::Value(Json::Text(reply.text));
    projected(
        context,
        &request.model,
        &request.prompt,
        Some(&prediction),
        Some(&reply.meta),
    )
}

fn endpoint_call(invocation: LegacyInvocation, scope: &RpcScope) -> Option<EndpointCall> {
    let (model, prompt, given_endpoint, instances) = match invocation {
        LegacyInvocation::CustomEndpoint {
            model,
            prompt,
            instances,
            ..
        } => (model, prompt, None, instances),
        LegacyInvocation::PrivateEndpoint {
            model,
            prompt,
            endpoint,
            instances,
            ..
        } => (model, prompt, Some(endpoint), instances),
        LegacyInvocation::Chat { .. }
        | LegacyInvocation::Text { .. }
        | LegacyInvocation::Vision { .. } => return None,
    };
    let endpoint = given_endpoint.unwrap_or_else(|| endpoint_path(scope, &model));
    Some(EndpointCall {
        model,
        prompt,
        endpoint,
        instances,
    })
}

async fn endpoint_completion<C>(
    client: &C,
    context: &LegacyContext<'_, '_>,
    scope: RpcScope,
    call: EndpointCall,
) -> Result<LegacyResult, PredictionError>
where
    C: PredictionClient,
{
    let EndpointCall {
        model,
        prompt,
        endpoint,
        instances,
    } = call;
    let request = EndpointPredictRequest {
        token: scope.token,
        project: scope.project,
        location: scope.location,
        endpoint,
        instances,
    };
    let reply = client.predict_endpoint(&request).await?;
    let Some(prediction) = reply.predictions.first() else {
        return Err(PredictionError::MissingPrediction);
    };
    let stripped = strip_output_separator(prediction);
    projected(context, &model, &prompt, Some(&stripped), Some(&reply.meta))
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
