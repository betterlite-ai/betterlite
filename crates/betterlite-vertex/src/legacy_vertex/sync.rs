use betterlite_core::message_utilities::response_streams::iterators::ModelResponseIterator;
use betterlite_models::decisions::{Extra, Json};
use secrecy::SecretString;
use uuid::Uuid;

use super::LegacyCompletion;
use super::error::LegacyVertexError;
use super::fake_response;
use super::plan::{LegacyPlan, LegacyRoute, endpoint_path};
use crate::endpoint_target::region::DEFAULT_LOCATION;
use crate::legacy_prediction::client::{
    EndpointPredictRequest, PredictionClient, PublisherPredictRequest,
};
use crate::legacy_prediction::error::PredictionError;
use crate::legacy_prediction::execution::LegacyContext;
use crate::legacy_prediction::projection::{LegacyResult, ProjectionInput, project};
use crate::legacy_prediction::records::{Prediction, strip_output_separator};
use crate::legacy_prediction::streamer::TextStreamer;
use crate::token_cache::probe::AccessToken;

struct Scope {
    token: SecretString,
    project: String,
    location: String,
}

// Everything a synchronous call needs from the environment, including the caller's id source
pub(crate) struct SyncDeps<'call, 'config, 'encoding> {
    pub(crate) context: &'call LegacyContext<'config, 'encoding>,
    pub(crate) access: AccessToken,
    pub(crate) identifier: fn() -> Uuid,
}

struct SyncCall<'call, 'config, 'encoding> {
    model: &'call str,
    prompt: &'call str,
    stream: bool,
    context: &'call LegacyContext<'config, 'encoding>,
    identifier: fn() -> Uuid,
}

#[derive(Clone, Copy)]
enum PublisherCall {
    Chat,
    Text { fake_stream: bool },
}

pub(crate) async fn execute<C>(
    client: &C,
    cached: Option<&C>,
    plan: LegacyPlan,
    deps: SyncDeps<'_, '_, '_>,
) -> Result<LegacyCompletion, LegacyVertexError>
where
    C: PredictionClient,
{
    let SyncDeps {
        context,
        access,
        identifier,
    } = deps;
    let call = SyncCall {
        model: &plan.model,
        prompt: &plan.prompt,
        stream: plan.stream,
        context,
        identifier,
    };
    match plan.route {
        // The source's vision branch calls no model, so it needs neither a project nor a location
        LegacyRoute::Vision => Ok(LegacyCompletion::Response(Box::new(result_of(
            &call, None,
        )?))),
        LegacyRoute::Chat { options } => {
            let target = cached.unwrap_or(client);
            let scope = scope_of(context, access);
            publish(target, &call, scope, options, PublisherCall::Chat).await
        }
        LegacyRoute::Text {
            options,
            fake_stream,
        } => {
            let target = cached.unwrap_or(client);
            let scope = scope_of(context, access);
            let kind = PublisherCall::Text { fake_stream };
            publish(target, &call, scope, options, kind).await
        }
        LegacyRoute::Custom { instances, .. } | LegacyRoute::Private { instances, .. } => {
            let request = endpoint_request(scope_of(context, access), call.model, instances);
            endpoint_reply(client, &call, request).await
        }
    }
}

fn scope_of(context: &LegacyContext<'_, '_>, access: AccessToken) -> Scope {
    let AccessToken { token, project } = access;
    let location = context.location.unwrap_or(DEFAULT_LOCATION);
    Scope {
        token,
        project: context.project.map_or(project, str::to_owned),
        location: location.to_owned(),
    }
}

async fn publish<C>(
    client: &C,
    call: &SyncCall<'_, '_, '_>,
    scope: Scope,
    options: Extra,
    kind: PublisherCall,
) -> Result<LegacyCompletion, LegacyVertexError>
where
    C: PredictionClient,
{
    let request = publisher_request(scope, call, options);
    let reply = match kind {
        PublisherCall::Chat if call.stream => {
            let stream = client.send_message_streaming_async(&request).await?;
            return Ok(LegacyCompletion::Prediction(stream));
        }
        PublisherCall::Chat => client.send_message_async(&request).await?,
        PublisherCall::Text { fake_stream: false } if call.stream => {
            let stream = client.predict_streaming_async(&request).await?;
            return Ok(LegacyCompletion::Prediction(stream));
        }
        PublisherCall::Text { .. } => client.predict_async(&request).await?,
    };
    let text = reply.text;
    if call.stream && matches!(kind, PublisherCall::Text { fake_stream: true }) {
        return fake_stream(call, text);
    }
    let prediction = Prediction::Value(Json::Text(text));
    Ok(LegacyCompletion::Response(Box::new(result_of(
        call,
        Some(&prediction),
    )?)))
}

async fn endpoint_reply<C>(
    client: &C,
    call: &SyncCall<'_, '_, '_>,
    request: EndpointPredictRequest,
) -> Result<LegacyCompletion, LegacyVertexError>
where
    C: PredictionClient,
{
    let reply = client.predict_endpoint(&request).await?;
    let Some(prediction) = reply.predictions.first() else {
        return Err(LegacyVertexError::Prediction(
            PredictionError::MissingPrediction,
        ));
    };
    let stripped = strip_output_separator(prediction);
    if !call.stream {
        return Ok(LegacyCompletion::Response(Box::new(result_of(
            call,
            Some(&stripped),
        )?)));
    }
    let Prediction::Value(Json::Text(text)) = &stripped else {
        return Err(LegacyVertexError::Prediction(
            PredictionError::WrongFieldType {
                field: "predictions",
            },
        ));
    };
    Ok(LegacyCompletion::Words(TextStreamer::new(text)))
}

fn endpoint_request(scope: Scope, model: &str, instances: Vec<Json>) -> EndpointPredictRequest {
    EndpointPredictRequest {
        endpoint: endpoint_path(&scope.project, &scope.location, model),
        token: scope.token,
        project: scope.project,
        location: scope.location,
        instances,
    }
}

// The source pops stream before every synchronous publisher call
fn publisher_request(
    scope: Scope,
    call: &SyncCall<'_, '_, '_>,
    mut options: Extra,
) -> PublisherPredictRequest {
    options.shift_remove("stream");
    PublisherPredictRequest {
        token: scope.token,
        project: scope.project,
        location: scope.location,
        model: call.model.to_owned(),
        prompt: call.prompt.to_owned(),
        options,
    }
}

fn fake_stream(
    call: &SyncCall<'_, '_, '_>,
    text: String,
) -> Result<LegacyCompletion, LegacyVertexError> {
    let prediction = Prediction::Value(Json::Text(text.clone()));
    let result = result_of(call, Some(&prediction))?;
    // The source's fake stream is that whole response, yielded once by B596's iterator
    let content = (!text.is_empty()).then_some(text.as_str());
    let response = fake_response::model_response(&result, content, call.identifier);
    let iterator = ModelResponseIterator::new(response, None)?;
    Ok(LegacyCompletion::FakeStream(iterator))
}

fn result_of(
    call: &SyncCall<'_, '_, '_>,
    prediction: Option<&Prediction>,
) -> Result<LegacyResult, LegacyVertexError> {
    let input = ProjectionInput {
        prediction,
        meta: None,
        model_is_language: call.context.model_is_language,
        prompt: call.prompt,
        model: call.model,
    };
    Ok(project(&input, call.context.services)?)
}
