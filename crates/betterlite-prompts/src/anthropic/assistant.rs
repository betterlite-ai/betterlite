use std::collections::HashSet;
use std::slice::from_ref;

use thiserror::Error;

use crate::anthropic_invokes::invoke::InvokeError;
use crate::anthropic_results::function_invoke;
use crate::arguments::json::{Json, Object};

use fields::read_thinking;
use fields::{has_server_tool_call, list_has_thinking, read_calls, read_content};
use tools::convert_tool_calls;

mod blocks;
pub(crate) mod fields;
mod tools;

const ASSISTANT: &str = "assistant";
const COMPACTION_BLOCKS: &str = "compaction_blocks";
const CONTENT: &str = "content";
const FUNCTION_CALL: &str = "function_call";
const ID: &str = "id";
const PROVIDER_SPECIFIC_FIELDS: &str = "provider_specific_fields";
const REDACTED_THINKING: &str = "redacted_thinking";
const ROLE: &str = "role";
const SERVER_TOOL_PREFIX: &str = "srvtoolu_";
const SERVER_TOOL_USE: &str = "server_tool_use";
const TEXT: &str = "text";
const THINKING: &str = "thinking";
const THINKING_BLOCKS: &str = "thinking_blocks";
const TOOL_CALLS: &str = "tool_calls";
const TOOL_RESULTS: &str = "tool_results";
const TOOL_RESULT_SUFFIX: &str = "_tool_result";
const TOOL_USE: &str = "tool_use";
const TYPE: &str = "type";
const WEB_SEARCH_RESULTS: &str = "web_search_results";

// The 1024-entry ceiling the tool invoke and the history sanitizer already put on one message's provider blocks
pub const MAX_PROVIDER_BLOCKS: usize = 1024;

#[derive(Debug, Error)]
pub enum AssistantError {
    #[error("message {index} field {field} has the wrong type")]
    Field { index: usize, field: &'static str },
    #[error("content block {block} of message {index} is not an object")]
    BlockNotObject { index: usize, block: usize },
    #[error("content block {block} of message {index} field {field} has the wrong type")]
    BlockField {
        index: usize,
        block: usize,
        field: &'static str,
    },
    #[error("message {index} lists more than {limit} {field}")]
    TooMany {
        index: usize,
        field: &'static str,
        limit: usize,
    },
    #[error(transparent)]
    ToolInvoke(#[from] InvokeError),
    #[error(transparent)]
    FunctionInvoke(#[from] function_invoke::InvokeError),
}

#[derive(Debug, Default)]
pub struct AssistantRun {
    blocks: Vec<Json>,
    unique_tool_ids: HashSet<String>,
}

struct ParsedAssistant<'input> {
    index: usize,
    fields: &'input Object,
    thinking: Option<Vec<Json>>,
    content: ContentField<'input>,
    calls: Option<&'input [Json]>,
    provider: Option<&'input Object>,
}

#[derive(Clone, Copy)]
enum ContentField<'input> {
    Missing,
    Text(&'input str),
    Blocks(&'input [Json]),
}

impl AssistantRun {
    #[must_use]
    pub fn into_message(self) -> Option<Json> {
        if self.blocks.is_empty() {
            return None;
        }
        Some(Json::Object(Object::from_iter([
            (ROLE.to_owned(), Json::Text(ASSISTANT.to_owned())),
            (CONTENT.to_owned(), Json::List(self.blocks)),
        ])))
    }

    pub fn append<Ids>(
        &mut self,
        fields: &Object,
        index: usize,
        new_id: &Ids,
    ) -> Result<(), AssistantError>
    where
        Ids: Fn() -> String,
    {
        let parsed = ParsedAssistant::read(fields, index)?;
        self.append_compaction(&parsed)?;
        if parsed.interleaved() {
            self.append_interleaved(&parsed)?;
        } else {
            self.append_sequential(&parsed)?;
        }
        self.append_function_call(&parsed, new_id)
    }

    fn append_compaction(&mut self, parsed: &ParsedAssistant<'_>) -> Result<(), AssistantError> {
        let Some(provider) = parsed.provider else {
            return Ok(());
        };
        let Some(Json::List(blocks)) = provider.get(COMPACTION_BLOCKS) else {
            return Ok(());
        };
        if blocks.len() > MAX_PROVIDER_BLOCKS {
            return Err(AssistantError::TooMany {
                index: parsed.index,
                field: COMPACTION_BLOCKS,
                limit: MAX_PROVIDER_BLOCKS,
            });
        }
        self.blocks.extend(blocks.iter().cloned());
        Ok(())
    }

    fn append_interleaved(&mut self, parsed: &ParsedAssistant<'_>) -> Result<(), AssistantError> {
        let (groups, regular) = tools::group_tool_invokes(convert_tool_calls(parsed)?);
        let thinking = parsed.thinking.as_deref().unwrap_or_default();
        let rounds = thinking.len().max(groups.len());
        for round in 0..rounds {
            if let Some(block) = thinking.get(round) {
                self.blocks.push(block.clone());
            }
            if let Some(group) = groups.get(round) {
                tools::push_unique(self, group);
            }
        }
        if let ContentField::Text(text) = parsed.content {
            blocks::push_text(self, text, parsed.fields);
        }
        for block in &regular {
            tools::push_unique(self, from_ref(block));
        }
        Ok(())
    }

    fn append_sequential(&mut self, parsed: &ParsedAssistant<'_>) -> Result<(), AssistantError> {
        if let Some(thinking) = &parsed.thinking
            && !list_has_thinking(parsed.content)
        {
            self.blocks.extend(thinking.iter().cloned());
        }
        match parsed.content {
            ContentField::Missing => {}
            ContentField::Text(text) => blocks::push_text(self, text, parsed.fields),
            ContentField::Blocks(items) => {
                blocks::append_content_blocks(self, items, parsed.index)?;
            }
        }
        if parsed.calls.is_some() {
            for block in &convert_tool_calls(parsed)? {
                tools::push_unique(self, from_ref(block));
            }
        }
        Ok(())
    }

    fn append_function_call<Ids>(
        &mut self,
        parsed: &ParsedAssistant<'_>,
        new_id: &Ids,
    ) -> Result<(), AssistantError>
    where
        Ids: Fn() -> String,
    {
        match parsed.fields.get(FUNCTION_CALL) {
            None | Some(Json::Null) => Ok(()),
            Some(Json::Object(function_call)) => {
                for block in function_invoke::to_anthropic_tool_use(function_call, new_id)? {
                    self.blocks.push(Json::Object(block));
                }
                Ok(())
            }
            Some(_) => Err(AssistantError::Field {
                index: parsed.index,
                field: FUNCTION_CALL,
            }),
        }
    }
}

impl<'input> ParsedAssistant<'input> {
    fn read(fields: &'input Object, index: usize) -> Result<Self, AssistantError> {
        let provider = match fields.get(PROVIDER_SPECIFIC_FIELDS) {
            None | Some(Json::Null) => None,
            Some(Json::Object(provider)) => Some(provider),
            Some(_) => {
                return Err(AssistantError::Field {
                    index,
                    field: PROVIDER_SPECIFIC_FIELDS,
                });
            }
        };
        Ok(Self {
            index,
            fields,
            thinking: read_thinking(fields, index)?,
            content: read_content(fields, index)?,
            calls: read_calls(fields, index)?,
            provider,
        })
    }

    fn interleaved(&self) -> bool {
        self.thinking.is_some()
            && has_server_tool_call(self.calls)
            && matches!(self.content, ContentField::Missing | ContentField::Text(_))
    }
}
