use super::convert::{
    ConvertError, TextArguments, TextCallWire, chat_messages, completion_request, prompt,
};
use super::error::{
    CallFacts, CallFailure, Raised, completion_failure, convert_failure, mock_timeout_failure,
    raised, stream_failure,
};
use super::session::{DeploymentFacts, TextSession, secret_text};
use crate::completion::setup::global_credentials;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::completion_chat_bridge::CompletionEndpoint;
use betterlite_openai::text_completion_execution::log::TracingCompletionLog;
use betterlite_openai::text_completion_execution::{CompletionCallAnswer, CompletionSend};
use betterlite_openai::text_completion_request::{CompletionRequest, PromptRefusal};
use betterlite_openai::text_completion_response::CompletionResponse;
use betterlite_openai::text_completion_stream::shaped_chunk;
use betterlite_openai::text_completion_transport::call::TextCompletionRequest as TransportRequest;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use betterlite_redact::secret::Redactor;
use futures_util::StreamExt as _;
use serde::Serialize;

// What main.py's text completion answers: the completion itself, the chunks of a stream, or the messages a chat provider gets.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum TextAnswer {
    Text { response: Box<CompletionResponse> },
    Chunks { chunks: Vec<String> },
    Chat { messages: Vec<Json> },
}

// main.py's text completion, from the resolved deployment to the answer Python turns into a TextCompletionResponse.
pub(crate) async fn call(
    session: &TextSession,
    wire: TextCallWire,
) -> Result<TextAnswer, CallFailure> {
    let TextCallWire {
        arguments,
        settings,
    } = wire;
    let globals = global_credentials(&settings);
    let api_key = secret_text(arguments.api_key.as_ref())
        .or_else(|| secret_text(settings.api_key.as_ref()))
        .or_else(|| secret_text(settings.openai_key.as_ref()));
    let requested = arguments.model.clone().unwrap_or_default();
    let resolved = session.resolve(&requested, &arguments, api_key.as_ref(), &globals)?;
    let provider = resolved.provider.slug().to_owned();
    let model = resolved
        .model
        .strip_prefix(&format!("{provider}/"))
        .map_or_else(|| resolved.model.clone(), str::to_owned);
    let facts = CallFacts {
        model: &model,
        provider: &provider,
    };
    // main.py's prompt dispatch refuses a value it cannot read as one user message before any provider runs
    if matches!(prompt(&arguments).refusal(), Some(PromptRefusal::Unmapped)) {
        return Err(convert_failure(ConvertError::Prompt, &facts));
    }
    let deployment = session.deployment(&DeploymentFacts {
        resolved: &resolved,
        arguments: &arguments,
        model: &model,
        provider: &provider,
        api_key,
        globals: &globals,
    })?;
    answer_for(session, &deployment, &arguments, &facts).await
}

// main.py's completion entry answers a mock itself, and sends everything else to the endpoint the deployment names
async fn answer_for(
    session: &TextSession,
    deployment: &ChatDeployment,
    arguments: &TextArguments,
    facts: &CallFacts<'_>,
) -> Result<TextAnswer, CallFailure> {
    // a mock timeout is the SDK mock entry's timeout error, raised before any provider runs
    if let Some(Json::Bool(true)) = arguments.extra.get("mock_timeout") {
        return Err(mock_timeout_failure(facts, &session.redactor));
    }
    // a mock answer comes from the SDK completion entry, which applies it before any provider runs
    let endpoint = if mocked(arguments) {
        CompletionEndpoint::Chat
    } else {
        session.executor.endpoint(deployment).await
    };
    match endpoint {
        CompletionEndpoint::Chat => {
            let messages =
                chat_messages(arguments).map_err(|error| convert_failure(error, facts))?;
            Ok(TextAnswer::Chat { messages })
        }
        CompletionEndpoint::Text => text_answer(session, deployment, arguments, facts).await,
    }
}

// main.py's completion entry answers a mock response itself, before it reaches any provider
fn mocked(arguments: &TextArguments) -> bool {
    arguments
        .extra
        .get("mock_response")
        .is_some_and(|value| !matches!(value, Json::Null))
}

// The text endpoint's call: the completion body, the provider's answer, and a stream drained into its chunks.
async fn text_answer(
    session: &TextSession,
    deployment: &ChatDeployment,
    arguments: &TextArguments,
    facts: &CallFacts<'_>,
) -> Result<TextAnswer, CallFailure> {
    let request = completion_request(arguments).map_err(|error| convert_failure(error, facts))?;
    let streamed = arguments.stream == Some(true);
    let mut transport = session
        .executor
        .text_transport(deployment, &request, streamed)
        .await
        .map_err(|failure| completion_failure(&failure, facts, &session.redactor))?;
    forward_parameters(&request, &mut transport);
    let mut logger = TracingCompletionLog;
    let answer = session
        .executor
        .send(CompletionSend {
            deployment,
            request: transport,
            streamed,
            redactor: &session.redactor,
            logger: &mut logger,
        })
        .await
        .map_err(|failure| completion_failure(&failure, facts, &session.redactor))?;
    match answer {
        CompletionCallAnswer::Buffered(buffered) => Ok(TextAnswer::Text {
            response: buffered.response,
        }),
        CompletionCallAnswer::Streamed(streamed) => Ok(TextAnswer::Chunks {
            chunks: collect_stream(streamed.stream, facts, &session.redactor).await?,
        }),
    }
}

// echo, best_of and suffix are completion parameters the proxy's own request model does not hold, so the transport drops them
fn forward_parameters(request: &CompletionRequest, transport: &mut TransportRequest) {
    for name in ["best_of", "echo", "suffix"] {
        if let Some(value) = request.passthrough.get(name) {
            transport
                .optional_params
                .insert(name.to_owned(), value.clone());
        }
    }
}

// The provider's stream, read to its end: a trailing usage-only chunk moves its usage onto the last chunk LiteLLM can convert.
async fn collect_stream(
    stream: CompletionStream,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> Result<Vec<String>, CallFailure> {
    let mut remaining = stream;
    let mut kept = Vec::new();
    while let Some(item) = remaining.next().await {
        let chunk = item.map_err(|error| stream_failure(&error, facts, redactor))?;
        if chunk.choices.is_empty() {
            if let Some(usage) = chunk.usage {
                if let Some(last) = kept.last_mut() {
                    last.usage = Some(usage);
                }
            }
            continue;
        }
        kept.push(chunk);
    }
    let mut chunks = Vec::with_capacity(kept.len());
    for chunk in kept {
        let shaped = shaped_chunk(chunk, "").map_err(|source| encode_failure(source, facts))?;
        let text =
            serde_json::to_string(&shaped).map_err(|source| encode_failure(source, facts))?;
        chunks.push(text);
    }
    Ok(chunks)
}

fn encode_failure(source: serde_json::Error, facts: &CallFacts<'_>) -> CallFailure {
    raised(
        Raised::InternalServer,
        source.to_string(),
        Some(facts.model.to_owned()),
        Some(facts.provider.to_owned()),
    )
}
