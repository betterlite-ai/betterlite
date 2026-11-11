use betterlite_http::user_url::validate::Resolve;
use serde::Deserialize;

use crate::arguments::json::Json;
use crate::bedrock_images::remote::RemoteImage;
use crate::bedrock_processor::conversation::{Conversion, converse_messages};
use crate::bedrock_tool_results::invoke::CachePoints;
use crate::bedrock_tools::capabilities::BedrockToolCapabilities;
use crate::bedrock_tools::name::ToolNameMap;
use crate::bedrock_tools::tools::{BedrockTools, bedrock_tools};
use crate::message_content::Message;

use super::error::PromptError;
use super::projection::{project_message, project_messages};

// LiteLLM's str | ChatCompletionAssistantMessage | None, where a bare string becomes one text block
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AssistantContinue {
    Text(String),
    Message(Message),
}

pub struct PromptSettings<'call> {
    pub model: Option<&'call str>,
    pub capabilities: BedrockToolCapabilities,
    pub user_continue_message: Option<&'call Message>,
    pub assistant_continue_message: Option<&'call AssistantContinue>,
    pub modify_params: bool,
}

pub struct PromptDeps<'deps, C, R, Ids> {
    pub cache_points: &'deps C,
    pub remote: &'deps RemoteImage<R>,
    pub new_id: &'deps Ids,
}

#[derive(Debug)]
pub struct BedrockPrompt {
    pub messages: Vec<Json>,
    pub tools: Vec<Json>,
    pub names: ToolNameMap,
}

fn project_assistant_continue(continue_message: &AssistantContinue) -> Json {
    match continue_message {
        AssistantContinue::Text(text) => Json::Text(text.clone()),
        AssistantContinue::Message(message) => project_message(message),
    }
}

pub async fn bedrock_prompt<C, R, Ids>(
    messages: &[Message],
    tools: &[Json],
    settings: &PromptSettings<'_>,
    deps: &PromptDeps<'_, C, R, Ids>,
) -> Result<BedrockPrompt, PromptError>
where
    C: CachePoints + Sync,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let BedrockTools { blocks, mut names } = bedrock_tools(tools, settings.capabilities)?;
    let continue_message = settings
        .assistant_continue_message
        .map(project_assistant_continue);
    let (projected, assistant_continue) =
        project_messages(messages, continue_message.as_ref(), &mut names);
    let user_continue = settings.user_continue_message.map(project_message);
    let conversion = Conversion {
        model: settings.model,
        cache_points: deps.cache_points,
        remote: deps.remote,
        user_continue_message: user_continue.as_ref(),
        assistant_continue_message: assistant_continue.as_ref(),
        modify_params: settings.modify_params,
    };
    let converted = converse_messages(&projected, &conversion, deps.new_id).await?;
    Ok(BedrockPrompt {
        messages: converted,
        tools: blocks,
        names,
    })
}
