use super::convert::{
    ConvertError, TextArguments, TextCallWire, chat_messages, completion_request,
};
use super::error::{
    CallFacts, CallFailure, Raised, RawSource, convert_failure, provider_failure, raised,
};
use super::session::{DeploymentFacts, TextSession, declared, secret_text};
use crate::completion::setup::global_credentials;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::completion_chat_bridge::CompletionEndpoint;
use betterlite_openai::text_completion_execution::log::TracingCompletionLog;
use betterlite_openai::text_completion_execution::{
    CompletionCallAnswer, CompletionExecutor, CompletionSend,
};
use betterlite_openai::text_completion_request::{
    CompletionPrompt, CompletionRequest, PromptRefusal,
};
use betterlite_openai::text_completion_response::CompletionResponse;
use betterlite_openai::text_completion_stream::shaped_chunk;
use betterlite_openai::text_completion_transport::call::TextCompletionRequest as TransportRequest;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use betterlite_redact::secret::Redactor;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use futures_util::StreamExt as _;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum TextAnswer {
    Text { response: Box<CompletionResponse> },
    Chunks { chunks: Vec<String> },
    Chat { messages: Vec<Json> },
}

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
    let stripped = resolved.model.strip_prefix(&format!("{provider}/"));
    let model = stripped.map_or_else(|| resolved.model.clone(), str::to_owned);
    let facts = CallFacts {
        model: &model,
        provider: &provider,
    };
    // main.py's prompt dispatch refuses a value it cannot read as one user message before any provider runs
    let completion =
        CompletionPrompt::from(serde_json::to_value(&arguments.prompt).unwrap_or_default());
    if matches!(completion.refusal(), Some(PromptRefusal::Unmapped)) {
        return Err(convert_failure(&ConvertError::Prompt, &facts));
    }
    let declared_facts = DeploymentFacts {
        resolved: &resolved,
        arguments: &arguments,
        model: &model,
        provider: &provider,
        api_key,
        globals: &globals,
    };
    let deployment = session.resolved(
        &declared(&declared_facts),
        declared_facts.model,
        declared_facts.provider,
        declared_facts.globals,
    )?;
    answer_for(session, &deployment, &arguments, &facts).await
}

// main.py's completion entry answers a mock itself, and sends everything else to the endpoint the deployment names
async fn answer_for(
    session: &TextSession,
    deployment: &ChatDeployment,
    arguments: &TextArguments,
    facts: &CallFacts<'_>,
) -> Result<TextAnswer, CallFailure> {
    // a mock entry answers before any provider runs, and the SDK completion entry on the chat path applies it
    let answered = ["mock_response", "mock_tool_calls", "mock_timeout"]
        .iter()
        .filter_map(|name| arguments.extra.get(*name))
        .any(|value| !matches!(value, Json::Null));
    if answered || session.executor.endpoint(deployment).await == CompletionEndpoint::Chat {
        return chat_messages(arguments, answered, facts.provider)
            .map(|messages| TextAnswer::Chat { messages })
            .map_err(|error| convert_failure(&error, facts));
    }
    sent_text(
        &session.executor,
        deployment,
        arguments,
        facts,
        &session.redactor,
    )
    .await
}

async fn sent_text(
    executor: &CompletionExecutor,
    deployment: &ChatDeployment,
    arguments: &TextArguments,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> Result<TextAnswer, CallFailure> {
    let request = completion_request(arguments).map_err(|error| convert_failure(&error, facts))?;
    let streamed = arguments.stream == Some(true);
    let mut transport = executor
        .text_transport(deployment, &request, streamed)
        .await
        .map_err(|failure| provider_failure(&RawSource::Completion(&failure), facts, redactor))?;
    forward_parameters(&request, &mut transport);
    let mut logger = TracingCompletionLog;
    let answer = executor
        .send(CompletionSend {
            deployment,
            request: transport,
            streamed,
            redactor,
            logger: &mut logger,
        })
        .await
        .map_err(|failure| provider_failure(&RawSource::Completion(&failure), facts, redactor))?;
    match answer {
        CompletionCallAnswer::Buffered(buffered) => Ok(TextAnswer::Text {
            response: buffered.response,
        }),
        CompletionCallAnswer::Streamed(chunked) => Ok(TextAnswer::Chunks {
            chunks: collect_stream(chunked.stream, facts, redactor).await?,
        }),
    }
}

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
    mut stream: CompletionStream,
    facts: &CallFacts<'_>,
    redactor: &Redactor,
) -> Result<Vec<String>, CallFailure> {
    let mut kept: Vec<StreamChunk> = Vec::new();
    while let Some(item) = stream.next().await {
        let chunk =
            item.map_err(|error| provider_failure(&RawSource::Stream(&error), facts, redactor))?;
        if !chunk.choices.is_empty() {
            kept.push(chunk);
            continue;
        }
        let Some(usage) = chunk.usage else {
            continue;
        };
        if let Some(last) = kept.last_mut() {
            last.usage = Some(usage);
        }
    }
    let mut chunks = Vec::with_capacity(kept.len());
    for chunk in kept {
        let shaped = shaped_chunk(chunk, "").map_err(|source| encode_failure(&source, facts))?;
        let text =
            serde_json::to_string(&shaped).map_err(|source| encode_failure(&source, facts))?;
        chunks.push(text);
    }
    Ok(chunks)
}

fn encode_failure(source: &serde_json::Error, facts: &CallFacts<'_>) -> CallFailure {
    raised(
        Raised::InternalServer,
        source.to_string(),
        Some(facts.model.to_owned()),
        Some(facts.provider.to_owned()),
    )
}
