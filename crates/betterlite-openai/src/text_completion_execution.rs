use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ::http::HeaderMap;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::Redactor;
use betterlite_streaming::stream_chunk_shaping::creation::system_instant;
use uuid::Uuid;

use crate::chat::deployment::ChatDeployment;
use crate::text_completion_policy::transform_completion_request;
use crate::text_completion_request::{CompletionPrompt, CompletionRequest, PromptRefusal};
use crate::text_completion_response::CompletionResponse;
use crate::text_completion_transport::call::{
    TextCompletionOutcome, TextCompletionRequest as TransportRequest,
};
use crate::text_completion_transport::logging::CompletionLogger;
use crate::text_completion_transport::{TextCompletionConfigs, TextCompletionTransport};

use self::catalog::{CompletionCatalog, CompletionRoute, route_for};
use self::failure::{CompletionFailure, INTERNAL_ERROR, transport_failure};
use self::http::{HttpCompletionConstructor, UpstreamFacts};

pub mod catalog;
pub mod failure;
pub mod http;
pub mod log;
mod tests;

const MISSING_PROMPT: &str =
    "Unmapped prompt format. The prompt must be a string, a list of strings or token ids.";

const INVALID_REQUEST_ERROR: &str = "invalid_request_error";

const NO_STREAM: &str = "betterlite does not stream text completions yet";

// main.py's text completion switch reaches these four provider tags, all sharing the one transform
const TRANSFORMS: TextCompletionConfigs = TextCompletionConfigs {
    open_ai: transform_completion_request,
    fireworks_ai: transform_completion_request,
    together_ai: transform_completion_request,
    text_completion_inception: transform_completion_request,
};

pub struct CompletionAnswer {
    pub response: Box<CompletionResponse>,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

impl CompletionAnswer {
    fn from_reply(response: CompletionResponse, facts: &Mutex<Option<UpstreamFacts>>) -> Self {
        let taken = facts.lock().unwrap_or_else(PoisonError::into_inner).take();
        let captured = taken.unwrap_or_default();
        Self {
            response: Box::new(response),
            upstream_headers: captured.headers,
            upstream_time: captured.elapsed,
        }
    }
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
        logger: &mut (dyn CompletionLogger + Send),
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
            configs: TRANSFORMS,
            now: system_instant,
            next_id: Uuid::new_v4,
        };
        let outcome = drive(&transport, transport_request, redactor, logger).await?;
        let TextCompletionOutcome::Buffered(reply) = outcome else {
            return Err(CompletionFailure::new(
                500,
                Some(INTERNAL_ERROR),
                NO_STREAM.to_owned(),
            ));
        };
        let response =
            serde_json::from_value::<CompletionResponse>(reply.response).map_err(|source| {
                CompletionFailure::new(
                    500,
                    Some(INTERNAL_ERROR),
                    format!("the provider completion could not be read: {source}"),
                )
            })?;
        Ok(CompletionAnswer::from_reply(response, &facts))
    }
}

// the prompt dispatch builds the messages the OpenAI handler transforms into the provider prompt
fn transport_request(
    deployment: &ChatDeployment,
    request: &CompletionRequest,
    model: &str,
) -> Result<TransportRequest, CompletionFailure> {
    let Some(messages) = request.prompt.messages() else {
        let message = String::from(MISSING_PROMPT);
        // main.py's own "Unmapped prompt format" Exception is the APIConnectionError shape; the empty list is the client's 400
        let empty = matches!(&request.prompt, CompletionPrompt::Empty);
        let error_type = match request.prompt.refusal() {
            Some(PromptRefusal::Unreadable) => Some(INTERNAL_ERROR),
            Some(PromptRefusal::Unmapped) | None => empty.then_some(INVALID_REQUEST_ERROR),
        };
        let status = if empty { 400 } else { 500 };
        return Err(CompletionFailure::new(status, error_type, message));
    };
    Ok(TransportRequest {
        model: Some(model.to_owned()),
        messages: Some(messages),
        api_key: deployment.api_key.clone(),
        api_base: Some(deployment.api_base.clone()),
        // a body timeout is the completion call's deadline; the deployment's applies only when the body names none
        timeout: request.timeout.unwrap_or(deployment.timeout_seconds),
        // main.py's text dispatch leaves the handler's organization at None, so no openai-organization header is sent
        organization: None,
        optional_params: request.optional_params().map_err(|error| {
            CompletionFailure::new(400, Some(INVALID_REQUEST_ERROR), error.to_string())
        })?,
        headers: None,
        acompletion: true,
    })
}

async fn drive(
    transport: &TextCompletionTransport<HttpCompletionConstructor>,
    request: TransportRequest,
    redactor: &Redactor,
    logger: &mut (dyn CompletionLogger + Send),
) -> Result<TextCompletionOutcome, CompletionFailure> {
    let begun = transport
        .begin(request, ProviderId::TextCompletionOpenai, &mut *logger)
        .map_err(|failure| transport_failure(failure, redactor))?;
    let sent = transport
        .send(&begun, None)
        .await
        .map_err(|failure| transport_failure(failure, redactor))?;
    transport
        .finish(&begun, sent, &mut *logger)
        .map_err(|failure| transport_failure(failure, redactor))
}
