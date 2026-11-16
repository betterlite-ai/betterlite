use std::error::Error;

use chrono::NaiveDate;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use tokio::sync::mpsc;

use super::constants::{DEFAULT_CHAT_MODEL, MAX_CHAT_MESSAGES, USAGE_AI_TEMPERATURE};
use super::error::{ChatError, UsageChatError};
use super::fetch_args::{
    FetchArguments, decode_tool_arguments, parse_csv_ids, resolve_fetch_kwargs,
};
use super::messages::ChatMessage;
use super::model::{ModelError, ModelReply, ModelRequest, ModelToolCall, UsageChatModel};
use super::parts::{UsageChatParts, UsageChatRequest};
use super::prompt::build_system_prompt;
use super::source::UsageDataSource;
use super::sse::{SseEvent, ToolCallStatus, sse_frame};
use super::summary::{EntityLabel, summarise_entity_data, summarise_usage_data};
use super::tools::{ToolDefinition, UsageTool, tools_for_role};

const THINKING: &str = "Thinking...";
const ANALYZING: &str = "Analyzing results...";
const GENERIC_ERROR: &str = "An internal error occurred. Please try again.";

pub async fn stream_usage_ai_chat<Model, Data, Clock>(
    parts: &UsageChatParts<Model, Data, Clock>,
    request: &UsageChatRequest<'_>,
    sink: &mpsc::Sender<String>,
) where
    Model: UsageChatModel,
    Data: UsageDataSource,
    Clock: Fn() -> NaiveDate + Sync,
{
    let model = request
        .model
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_CHAT_MODEL);
    let skip = request.messages.len().saturating_sub(MAX_CHAT_MESSAGES);
    let mut chat = vec![ChatMessage::System {
        content: build_system_prompt(request.is_admin, (parts.clock)()),
    }];
    chat.extend(request.messages.iter().skip(skip).cloned());
    match run_chat(parts, model, &mut chat, request, sink).await {
        Ok(()) | Err(ChatError::ClientDisconnected(_)) => {}
        Err(error) => {
            tracing::error!("AI usage chat failed: {error}");
            let frame = sse_frame(&SseEvent::Error {
                message: GENERIC_ERROR.to_owned(),
            });
            let _sent: Result<(), mpsc::error::SendError<String>> = sink.send(frame).await;
        }
    }
}

async fn run_chat<Model, Data, Clock>(
    parts: &UsageChatParts<Model, Data, Clock>,
    model: &str,
    chat: &mut Vec<ChatMessage>,
    request: &UsageChatRequest<'_>,
    sink: &mpsc::Sender<String>,
) -> Result<(), ChatError<Model::Error>>
where
    Model: UsageChatModel,
    Data: UsageDataSource,
    Clock: Sync,
{
    let reply = request_reply(&parts.model, model, chat.as_slice(), request.is_admin, sink).await?;
    if reply.tool_calls.is_empty() {
        if let Some(content) = reply.content.filter(|text| !text.is_empty()) {
            sink.send(sse_frame(&SseEvent::Chunk { content })).await?;
        }
        sink.send(sse_frame(&SseEvent::Done)).await?;
        return Ok(());
    }
    chat.push(ChatMessage::AssistantToolCalls {
        content: reply.content,
        tool_calls: reply.tool_calls.clone(),
    });
    for call in &reply.tool_calls {
        process_tool_call::<Data, Model::Error>(&parts.data, call, chat, request, sink).await?;
    }
    stream_final_response(&parts.model, model, chat.as_slice(), sink).await?;
    sink.send(sse_frame(&SseEvent::Done)).await?;
    Ok(())
}

async fn request_reply<Model>(
    model: &Model,
    name: &str,
    messages: &[ChatMessage],
    is_admin: bool,
    sink: &mpsc::Sender<String>,
) -> Result<ModelReply, ChatError<Model::Error>>
where
    Model: UsageChatModel,
{
    let thinking = SseEvent::Status {
        message: THINKING.to_owned(),
    };
    sink.send(sse_frame(&thinking)).await?;
    let tools = tools_for_role(is_admin);
    let request = model_request(name, messages, Some(tools.as_slice()));
    model
        .complete(&request)
        .await
        .map_err(|source| ChatError::Model(ModelError::Completion { source }))
}

async fn process_tool_call<Data, Fault>(
    data: &Data,
    call: &ModelToolCall,
    chat: &mut Vec<ChatMessage>,
    request: &UsageChatRequest<'_>,
    sink: &mpsc::Sender<String>,
) -> Result<(), ChatError<Fault>>
where
    Data: UsageDataSource,
    Fault: Error + 'static,
{
    let arguments = decode_tool_arguments(&call.arguments)?;
    let Some(tool) =
        UsageTool::from_name(&call.name).filter(|tool| tool.allowed_for(request.is_admin))
    else {
        chat.push(ChatMessage::Tool {
            tool_call_id: call.id.clone(),
            content: format!("Tool not available: {}", call.name),
        });
        return Ok(());
    };
    let running = tool_event(tool, &call.name, &arguments, ToolCallStatus::Running);
    sink.send(sse_frame(&running)).await?;
    let (content, status) = match execute_tool(data, tool, &arguments, request).await {
        Ok(text) => (text, ToolCallStatus::Complete),
        Err(error) => {
            tracing::error!("Tool {} failed: {error}", call.name);
            (
                format!("Error fetching {}. Please try again.", tool.label()),
                ToolCallStatus::Error,
            )
        }
    };
    let finished = tool_event(tool, &call.name, &arguments, status);
    sink.send(sse_frame(&finished)).await?;
    chat.push(ChatMessage::Tool {
        tool_call_id: call.id.clone(),
        content,
    });
    Ok(())
}

async fn execute_tool<Data>(
    data: &Data,
    tool: UsageTool,
    arguments: &IndexMap<String, String>,
    request: &UsageChatRequest<'_>,
) -> Result<String, UsageChatError>
where
    Data: UsageDataSource,
{
    let fetch = resolve_fetch_kwargs(tool, arguments, request.user_id, request.is_admin)?;
    let page = match &fetch {
        FetchArguments::Global {
            start_date,
            end_date,
            user_id,
        } => {
            data.global(start_date, end_date, user_id.as_deref())
                .await?
        }
        FetchArguments::Team {
            start_date,
            end_date,
            team_ids,
        } => {
            let ids = parse_csv_ids(team_ids.as_deref());
            data.teams(start_date, end_date, ids.as_deref()).await?
        }
        FetchArguments::Tag {
            start_date,
            end_date,
            tags,
        } => {
            let ids = parse_csv_ids(tags.as_deref());
            data.tags(start_date, end_date, ids.as_deref()).await?
        }
    };
    match tool {
        UsageTool::Usage => summarise_usage_data(&page),
        UsageTool::Team => summarise_entity_data(&page, EntityLabel::Team),
        UsageTool::Tag => summarise_entity_data(&page, EntityLabel::Tag),
    }
}

async fn stream_final_response<Model>(
    model: &Model,
    name: &str,
    messages: &[ChatMessage],
    sink: &mpsc::Sender<String>,
) -> Result<(), ChatError<Model::Error>>
where
    Model: UsageChatModel,
{
    let request = model_request(name, messages, None);
    let analyzing = SseEvent::Status {
        message: ANALYZING.to_owned(),
    };
    sink.send(sse_frame(&analyzing)).await?;
    let opened = model
        .stream(&request)
        .await
        .map_err(|source| ModelError::Stream { source })?;
    let mut stream = Box::pin(opened);
    while let Some(delta) = stream.next().await {
        let content = delta.map_err(|source| ModelError::Stream { source })?;
        if !content.is_empty() {
            sink.send(sse_frame(&SseEvent::Chunk { content })).await?;
        }
    }
    Ok(())
}

fn model_request<'text>(
    name: &'text str,
    messages: &'text [ChatMessage],
    tools: Option<&'text [ToolDefinition]>,
) -> ModelRequest<'text> {
    ModelRequest {
        model: name,
        messages,
        tools,
        temperature: USAGE_AI_TEMPERATURE,
    }
}

fn tool_event(
    tool: UsageTool,
    name: &str,
    arguments: &IndexMap<String, String>,
    status: ToolCallStatus,
) -> SseEvent {
    SseEvent::ToolCall {
        tool_name: name.to_owned(),
        tool_label: tool.label().to_owned(),
        arguments: arguments.clone(),
        status,
    }
}
