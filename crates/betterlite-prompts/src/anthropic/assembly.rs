use thiserror::Error;

use crate::anthropic::assistant::AssistantRun;
use crate::anthropic::error::AnthropicPromptError;
use crate::anthropic_results::tool_result::FileBlocks;
use crate::anthropic_user_blocks::prelude::prepare;
use crate::anthropic_user_blocks::user_run::UserBlockBuilder;
use crate::arguments::json::{Json, Object};
use crate::media::inline::RemoteMediaFetcher;
use crate::message_content::Message;
use crate::system_placement::anthropic_system::anthropic_system_messages;

const ASSISTANT: &str = "assistant";
const CONTENT: &str = "content";
const ROLE: &str = "role";
const SYSTEM: &str = "system";
const TEXT: &str = "text";
const TYPE: &str = "type";

#[derive(Debug, Error)]
pub enum AssemblyError {
    #[error("message at index {index} is not an object")]
    MessageNotObject { index: usize },
    #[error("message at index {index} has no role")]
    MissingRole { index: usize },
    #[error("message at index {index} has a role that is not a string")]
    RoleNotText { index: usize },
    #[error("message at index {index} has role {role}, which Anthropic does not accept")]
    UnsupportedRole { index: usize, role: String },
    #[error("the system message at index {index} cannot be read as an OpenAI message")]
    SystemMessage {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("content block {block} of the last assistant message has no string {field}")]
    TrailingBlock { block: usize, field: &'static str },
}

struct NextTurn(usize);

struct RoleAt<'input> {
    index: usize,
    message: &'input Json,
    fields: &'input Object,
    name: &'input str,
}

pub struct PromptDeps<'deps, Source, Blocks, Ids> {
    pub fetcher: &'deps Source,
    pub blocks: &'deps Blocks,
    pub new_id: &'deps Ids,
}

pub async fn anthropic_messages_pt<Source, Blocks, Ids>(
    messages: &[Json],
    model: &str,
    llm_provider: &str,
    modify_params: bool,
    deps: &PromptDeps<'_, Source, Blocks, Ids>,
) -> Result<Vec<Json>, AnthropicPromptError<Blocks::Failure>>
where
    Source: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    let prepared = prepare(messages, model, llm_provider, modify_params)?;
    let mut assembled = deps
        .assemble(&prepared.messages, prepared.force_base64)
        .await?;
    strip_trailing_whitespace(&mut assembled)?;
    Ok(assembled)
}

impl<Source, Blocks, Ids> PromptDeps<'_, Source, Blocks, Ids>
where
    Source: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    async fn assemble(
        &self,
        messages: &[Json],
        force_base64: bool,
    ) -> Result<Vec<Json>, AnthropicPromptError<Blocks::Failure>> {
        let builder = UserBlockBuilder {
            force_base64,
            fetcher: self.fetcher,
            blocks: self.blocks,
            new_id: self.new_id,
        };
        let mut assembled = Vec::new();
        let mut index = 0;
        while let Some(role) = role_at(messages, index)? {
            index = self
                .convert_turn(messages, role, &builder, &mut assembled)
                .await?
                .0;
        }
        Ok(assembled)
    }

    async fn convert_turn(
        &self,
        messages: &[Json],
        role: RoleAt<'_>,
        builder: &UserBlockBuilder<'_, Source, Blocks, Ids>,
        assembled: &mut Vec<Json>,
    ) -> Result<NextTurn, AnthropicPromptError<Blocks::Failure>> {
        let start = role.index;
        let user = builder.convert(messages, start).await?;
        if let Some(message) = user.message() {
            assembled.push(message);
        }
        let mut index = user.next;
        let mut assistant = AssistantRun::default();
        while let Some(next) = role_at(messages, index)? {
            if next.name != ASSISTANT {
                break;
            }
            assistant.append(next.fields, index, self.new_id)?;
            index = index.saturating_add(1);
        }
        if let Some(message) = assistant.into_message() {
            assembled.push(message);
        }
        index = append_system_messages(messages, index, assembled)?;
        if index == start {
            return Err(AnthropicPromptError::Assembly(
                AssemblyError::UnsupportedRole {
                    index,
                    role: role.name.to_owned(),
                },
            ));
        }
        Ok(NextTurn(index))
    }
}

fn append_system_messages(
    messages: &[Json],
    mut index: usize,
    assembled: &mut Vec<Json>,
) -> Result<usize, AssemblyError> {
    while let Some(role) = role_at(messages, index)? {
        if role.name != SYSTEM {
            break;
        }
        assembled.extend(system_message(role.message, index)?);
        index = index.saturating_add(1);
    }
    Ok(index)
}

fn system_message(message: &Json, index: usize) -> Result<Vec<Json>, AssemblyError> {
    let decoded_value = serde_json::to_value(message)
        .map_err(|source| AssemblyError::SystemMessage { index, source })?;
    let decoded = serde_json::from_value::<Message>(decoded_value)
        .map_err(|source| AssemblyError::SystemMessage { index, source })?;
    let mut wire_messages = Vec::new();
    for wire in anthropic_system_messages(&decoded) {
        let wire_value = serde_json::to_value(&wire)
            .map_err(|source| AssemblyError::SystemMessage { index, source })?;
        let json = serde_json::from_value::<Json>(wire_value)
            .map_err(|source| AssemblyError::SystemMessage { index, source })?;
        wire_messages.push(json);
    }
    Ok(wire_messages)
}

fn role_at(messages: &[Json], index: usize) -> Result<Option<RoleAt<'_>>, AssemblyError> {
    let Some(message) = messages.get(index) else {
        return Ok(None);
    };
    let Some(fields) = message.as_object() else {
        return Err(AssemblyError::MessageNotObject { index });
    };
    match fields.get(ROLE) {
        None => Err(AssemblyError::MissingRole { index }),
        Some(Json::Text(name)) => Ok(Some(RoleAt {
            index,
            message,
            fields,
            name,
        })),
        Some(_) => Err(AssemblyError::RoleNotText { index }),
    }
}

pub(crate) fn strip_trailing_whitespace(messages: &mut [Json]) -> Result<(), AssemblyError> {
    let Some(last) = messages.last_mut() else {
        return Ok(());
    };
    let Json::Object(fields) = last else {
        return Ok(());
    };
    if fields.get(ROLE).and_then(Json::as_str) != Some(ASSISTANT) {
        return Ok(());
    }
    match fields.get_mut(CONTENT) {
        Some(Json::Text(text)) => *text = text.trim_end_matches(is_rstrip_whitespace).to_owned(),
        Some(Json::List(blocks)) => strip_text_blocks(blocks)?,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => {}
    }
    Ok(())
}

fn strip_text_blocks(blocks: &mut [Json]) -> Result<(), AssemblyError> {
    for (position, block) in blocks.iter_mut().enumerate() {
        let Json::Object(fields) = block else {
            continue;
        };
        let Some(kind) = fields.get(TYPE) else {
            return Err(AssemblyError::TrailingBlock {
                block: position,
                field: TYPE,
            });
        };
        if kind.as_str() != Some(TEXT) {
            continue;
        }
        match fields.get_mut(TEXT) {
            Some(Json::Text(text)) => {
                *text = text.trim_end_matches(is_rstrip_whitespace).to_owned();
            }
            Some(_) | None => {
                return Err(AssemblyError::TrailingBlock {
                    block: position,
                    field: TEXT,
                });
            }
        }
    }
    Ok(())
}

// Python's str.rstrip() also removes the four information separators that Rust's White_Space set omits
const fn is_rstrip_whitespace(character: char) -> bool {
    match character {
        '\u{1c}'..='\u{1f}' => true,
        _ => character.is_whitespace(),
    }
}
