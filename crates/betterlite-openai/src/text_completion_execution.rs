use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::Redactor;
use betterlite_streaming::stream_chunk_shaping::creation::system_instant;
use http::HeaderMap;
use uuid::Uuid;

use crate::chat::deployment::ChatDeployment;
use crate::text_completion_policy::{TextCompletionConfigs, transform_completion_request};
use crate::text_completion_request::CompletionRequest;
use crate::text_completion_response::CompletionResponse;
use crate::text_completion_transport::TextCompletionTransport;
use crate::text_completion_transport::call::{
    TextCompletionOutcome, TextCompletionRequest as TransportRequest,
};
use crate::text_completion_transport::logging::CompletionLogger;

use self::catalog::{CompletionCatalog, CompletionRoute, route_for};
use self::failure::{CompletionFailure, request_failure, transport_failure};
use self::http::{HttpCompletionConstructor, UpstreamFacts};

pub mod catalog;
pub mod failure;
pub mod http;
pub mod log;
mod tests;

const MISSING_PROMPT: &str =
    "Unmapped prompt format. The prompt must be a string, a list of strings or token ids.";

pub struct CompletionAnswer {
    pub response: Box<CompletionResponse>,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

pub struct CompletionExecutor {
    pub client: ProxiedClient,
    pub catalog: CompletionCatalog,
}

impl CompletionExecutor {
    // main.py's switch: a non-chat OpenAI model goes to the text-completion-openai identity
    pub async fn complete(
        &self,
        deployment: &ChatDeployment,
        request: &CompletionRequest,
        redactor: &Redactor,
        logger: &mut dyn CompletionLogger,
    ) -> Result<CompletionAnswer, CompletionFailure> {
        let route = route_for(deployment, &self.catalog);
        let CompletionRoute::Text { model } = &route else {
            return Err(CompletionFailure::route(&route));
        };
        let transport_request = transport_request(deployment, request, model)?;
        let facts = Arc::new(Mutex::new(None));
        let transport = TextCompletionTransport {
            constructor: HttpCompletionConstructor {
                client: self.client.clone(),
                authorization: deployment.credential(),
                facts: Arc::clone(&facts),
            },
            configs: completion_configs(),
            now: system_instant,
            next_id: Uuid::new_v4,
        };
        let outcome = drive(&transport, transport_request, redactor, logger).await?;
        let TextCompletionOutcome::Buffered(reply) = outcome else {
            return Err(CompletionFailure::internal(String::from(
                "betterlite does not stream text completions yet",
            )));
        };
        let response =
            serde_json::from_value::<CompletionResponse>(reply.response).map_err(|source| {
                CompletionFailure::internal(format!(
                    "the provider completion could not be read: {source}"
                ))
            })?;
        let facts = facts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_default();
        Ok(CompletionAnswer {
            response: Box::new(response),
            upstream_headers: facts.headers,
            upstream_time: facts.elapsed,
        })
    }
}

// the prompt dispatch builds the messages the OpenAI handler transforms into the provider prompt
fn transport_request(
    deployment: &ChatDeployment,
    request: &CompletionRequest,
    model: &str,
) -> Result<TransportRequest, CompletionFailure> {
    let Some(messages) = request.prompt.messages() else {
        return Err(CompletionFailure::internal(String::from(MISSING_PROMPT)));
    };
    Ok(TransportRequest {
        model: Some(model.to_owned()),
        messages: Some(messages),
        api_key: deployment.api_key.clone(),
        api_base: Some(deployment.api_base.clone()),
        timeout: deployment.timeout_seconds,
        organization: deployment.organization.clone(),
        optional_params: request.optional_params().map_err(request_failure)?,
        headers: None,
        acompletion: true,
    })
}

async fn drive(
    transport: &TextCompletionTransport<HttpCompletionConstructor>,
    request: TransportRequest,
    redactor: &Redactor,
    logger: &mut dyn CompletionLogger,
) -> Result<TextCompletionOutcome, CompletionFailure> {
    let begun = transport
        .begin(request, ProviderId::TextCompletionOpenai, logger)
        .map_err(|failure| transport_failure(failure, redactor))?;
    let sent = transport
        .send(&begun, None)
        .await
        .map_err(|failure| transport_failure(failure, redactor))?;
    transport
        .finish(&begun, sent, logger)
        .map_err(|failure| transport_failure(failure, redactor))
}

#[must_use]
pub fn completion_configs() -> TextCompletionConfigs {
    TextCompletionConfigs {
        open_ai: transform_completion_request,
        fireworks_ai: transform_completion_request,
        together_ai: transform_completion_request,
        text_completion_inception: transform_completion_request,
    }
}
