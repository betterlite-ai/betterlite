use chrono::NaiveDate;
use futures_util::StreamExt;
use indexmap::IndexMap;
use tokio::sync::mpsc;

use super::constants::{DEFAULT_CHAT_MODEL, MAX_CHAT_MESSAGES, USAGE_AI_TEMPERATURE};
use super::error::UsageChatError;
use super::fetch_args::{
    FetchArguments, decode_tool_arguments, parse_csv_ids, resolve_fetch_kwargs,
};
use super::messages::ChatMessage;
use super::model::{ModelReply, ModelRequest, ModelToolCall, UsageChatModel};
use super::parts::{UsageChatParts, UsageChatRequest};
use super::prompt::build_system_prompt;
use super::source::UsageDataSource;
use super::sse::{SseEvent, ToolCallStatus, sse_frame};
use super::summary::{EntityLabel, summarise_entity_data, summarise_usage_data};
use super::tools::{UsageTool, tools_for_role};

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
        Ok(()) | Err(UsageChatError::ClientDisconnected) => {}
        Err(error) => {
            tracing::error!("AI usage chat failed: {error}");
            let frame = sse_frame(&SseEvent::Error {
                message: GENERIC_ERROR.to_owned(),
            });
            let _sent: Result<(), mpsc::error::SendError<String>> = sink.send(frame).await;
        }
    }
}

async fn run_chat<Model: UsageChatModel, Data: UsageDataSource, Clock>(
    parts: &UsageChatParts<Model, Data, Clock>,
    model: &str,
    chat: &mut Vec<ChatMessage>,
    request: &UsageChatRequest<'_>,
    sink: &mpsc::Sender<String>,
) -> Result<(), UsageChatError> {
    send_event(
        sink,
        &SseEvent::Status {
            message: THINKING.to_owned(),
        },
    )
    .await?;
    let tools = tools_for_role(request.is_admin);
    let reply = parts
        .model
        .complete(&ModelRequest {
            model,
            messages: chat.as_slice(),
            tools: Some(tools.as_slice()),
            temperature: USAGE_AI_TEMPERATURE,
        })
        .await?;
    if reply.tool_calls.is_empty() {
        if let Some(content) = reply.content.filter(|text| !text.is_empty()) {
            send_event(sink, &SseEvent::Chunk { content }).await?;
        }
        return send_event(sink, &SseEvent::Done).await;
    }
    let ModelReply {
        content,
        tool_calls,
    } = reply;
    chat.push(ChatMessage::AssistantToolCalls {
        content,
        tool_calls: tool_calls.clone(),
    });
    for call in tool_calls {
        process_tool_call(&parts.data, &call, chat, request, sink).await?;
    }
    stream_final_response(&parts.model, model, chat.as_slice(), sink).await?;
    send_event(sink, &SseEvent::Done).await
}

async fn process_tool_call<Data: UsageDataSource>(
    data: &Data,
    call: &ModelToolCall,
    chat: &mut Vec<ChatMessage>,
    request: &UsageChatRequest<'_>,
    sink: &mpsc::Sender<String>,
) -> Result<(), UsageChatError> {
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
    send_event(sink, &running).await?;
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
    send_event(sink, &finished).await?;
    chat.push(ChatMessage::Tool {
        tool_call_id: call.id.clone(),
        content,
    });
    Ok(())
}

async fn execute_tool<Data: UsageDataSource>(
    data: &Data,
    tool: UsageTool,
    arguments: &IndexMap<String, String>,
    request: &UsageChatRequest<'_>,
) -> Result<String, UsageChatError> {
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

async fn stream_final_response<Model: UsageChatModel>(
    model: &Model,
    name: &str,
    messages: &[ChatMessage],
    sink: &mpsc::Sender<String>,
) -> Result<(), UsageChatError> {
    send_event(
        sink,
        &SseEvent::Status {
            message: ANALYZING.to_owned(),
        },
    )
    .await?;
    let request = ModelRequest {
        model: name,
        messages,
        tools: None,
        temperature: USAGE_AI_TEMPERATURE,
    };
    let mut stream = Box::pin(model.stream(&request).await?);
    while let Some(delta) = stream.next().await {
        let content = delta?;
        if !content.is_empty() {
            send_event(sink, &SseEvent::Chunk { content }).await?;
        }
    }
    Ok(())
}

async fn send_event(sink: &mpsc::Sender<String>, event: &SseEvent) -> Result<(), UsageChatError> {
    match sink.send(sse_frame(event)).await {
        Ok(()) => Ok(()),
        Err(_) => Err(UsageChatError::ClientDisconnected),
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
