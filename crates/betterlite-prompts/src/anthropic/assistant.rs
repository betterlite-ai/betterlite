use std::collections::HashSet;
use std::mem::take;
use std::slice::from_ref;

use thiserror::Error;

use crate::anthropic_history::thinking::drop_unsignable;
use crate::anthropic_invokes::cache_control::add_cache_control_to_content;
use crate::anthropic_invokes::invoke::{InvokeError, MAX_TOOL_CALLS, to_anthropic_tool_invokes};
use crate::anthropic_results::function_invoke::{self, to_anthropic_tool_use};
use crate::arguments::json::{Json, Object};

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
const TOOL_RESULT_SUFFIX: &str = "_tool_result";
const TOOL_RESULTS: &str = "tool_results";
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
    pub fn message(&self) -> Option<Json> {
        if self.blocks.is_empty() {
            return None;
        }
        let content = self.blocks.clone();
        Some(Json::Object(Object::from_iter([
            (ROLE.to_owned(), Json::Text(ASSISTANT.to_owned())),
            (CONTENT.to_owned(), Json::List(content)),
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
        let (groups, regular) = group_tool_invokes(convert_tool_calls(parsed)?);
        let thinking = parsed.thinking.as_deref().unwrap_or_default();
        let rounds = thinking.len().max(groups.len());
        for round in 0..rounds {
            if let Some(block) = thinking.get(round) {
                self.blocks.push(block.clone());
            }
            if let Some(group) = groups.get(round) {
                self.push_unique(group);
            }
        }
        if let ContentField::Text(text) = parsed.content {
            self.push_text(text, parsed.fields);
        }
        for block in &regular {
            self.push_unique(from_ref(block));
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
            ContentField::Text(text) => self.push_text(text, parsed.fields),
            ContentField::Blocks(blocks) => self.append_content_blocks(blocks, parsed.index)?,
        }
        if parsed.calls.is_some() {
            for block in &convert_tool_calls(parsed)? {
                self.push_unique(from_ref(block));
            }
        }
        Ok(())
    }

    fn append_content_blocks(
        &mut self,
        blocks: &[Json],
        index: usize,
    ) -> Result<(), AssistantError> {
        for (position, block) in blocks.iter().enumerate() {
            let Some(fields) = block.as_object() else {
                return Err(AssistantError::BlockNotObject {
                    index,
                    block: position,
                });
            };
            match block_kind(fields, index, position)?.as_str() {
                THINKING => self.push_thinking_part(fields, block, index, position)?,
                TEXT => self.push_text_part(fields, index, position)?,
                SERVER_TOOL_USE => self.blocks.push(block.clone()),
                kind if kind.ends_with(TOOL_RESULT_SUFFIX) => self.blocks.push(block.clone()),
                _ => {}
            }
        }
        Ok(())
    }

    fn push_thinking_part(
        &mut self,
        fields: &Object,
        block: &Json,
        index: usize,
        position: usize,
    ) -> Result<(), AssistantError> {
        let text = match fields.get(THINKING) {
            None => "",
            Some(Json::Text(text)) => text.as_str(),
            Some(_) => {
                return Err(AssistantError::BlockField {
                    index,
                    block: position,
                    field: THINKING,
                });
            }
        };
        if text.is_empty() {
            return Ok(());
        }
        if drop_unsignable(from_ref(block)).is_empty() {
            return Ok(());
        }
        self.blocks.push(block.clone());
        Ok(())
    }

    fn push_text_part(
        &mut self,
        fields: &Object,
        index: usize,
        position: usize,
    ) -> Result<(), AssistantError> {
        let text = match fields.get(TEXT) {
            None => "",
            Some(Json::Text(text)) => text.as_str(),
            Some(_) => {
                return Err(AssistantError::BlockField {
                    index,
                    block: position,
                    field: TEXT,
                });
            }
        };
        if text.is_empty() {
            return Ok(());
        }
        let element = Object::from_iter([
            (TYPE.to_owned(), Json::Text(TEXT.to_owned())),
            (TEXT.to_owned(), Json::Text(text.to_owned())),
        ]);
        self.blocks
            .push(Json::Object(add_cache_control_to_content(element, fields)));
        Ok(())
    }

    fn push_text(&mut self, text: &str, fields: &Object) {
        if text.is_empty() {
            return;
        }
        let element = Object::from_iter([
            (TYPE.to_owned(), Json::Text(TEXT.to_owned())),
            (TEXT.to_owned(), Json::Text(text.to_owned())),
        ]);
        self.blocks
            .push(Json::Object(add_cache_control_to_content(element, fields)));
    }

    fn push_unique(&mut self, blocks: &[Object]) {
        for block in blocks {
            let Some(id) = block
                .get(ID)
                .and_then(Json::as_str)
                .filter(|id| !id.is_empty())
            else {
                self.blocks.push(Json::Object(block.clone()));
                continue;
            };
            if self.unique_tool_ids.insert(id.to_owned()) {
                self.blocks.push(Json::Object(block.clone()));
            }
        }
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
                for block in to_anthropic_tool_use(function_call, new_id)? {
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
        Ok(Self {
            index,
            fields,
            thinking: read_thinking_blocks(fields, index)?,
            content: read_content(fields, index)?,
            calls: read_tool_calls(fields, index)?,
            provider: read_provider_fields(fields, index)?,
        })
    }

    fn interleaved(&self) -> bool {
        self.thinking.is_some()
            && has_server_tool_call(self.calls)
            && matches!(self.content, ContentField::Missing | ContentField::Text(_))
    }
}

fn read_thinking_blocks(
    fields: &Object,
    index: usize,
) -> Result<Option<Vec<Json>>, AssistantError> {
    match fields.get(THINKING_BLOCKS) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(blocks)) => {
            if blocks.len() > MAX_PROVIDER_BLOCKS {
                return Err(AssistantError::TooMany {
                    index,
                    field: THINKING_BLOCKS,
                    limit: MAX_PROVIDER_BLOCKS,
                });
            }
            if let Some(position) = blocks
                .iter()
                .position(|block| !matches!(block, Json::Object(_)))
            {
                return Err(AssistantError::BlockNotObject {
                    index,
                    block: position,
                });
            }
            Ok(Some(drop_unsignable(blocks)))
        }
        Some(_) => Err(AssistantError::Field {
            index,
            field: THINKING_BLOCKS,
        }),
    }
}

fn read_content(fields: &Object, index: usize) -> Result<ContentField<'_>, AssistantError> {
    match fields.get(CONTENT) {
        None | Some(Json::Null) => Ok(ContentField::Missing),
        Some(Json::Text(text)) => Ok(ContentField::Text(text)),
        Some(Json::List(blocks)) => Ok(ContentField::Blocks(blocks)),
        Some(_) => Err(AssistantError::Field {
            index,
            field: CONTENT,
        }),
    }
}

fn read_tool_calls(fields: &Object, index: usize) -> Result<Option<&[Json]>, AssistantError> {
    match fields.get(TOOL_CALLS) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(calls)) => {
            if calls.len() > MAX_TOOL_CALLS {
                return Err(AssistantError::TooMany {
                    index,
                    field: TOOL_CALLS,
                    limit: MAX_TOOL_CALLS,
                });
            }
            Ok(Some(calls))
        }
        Some(_) => Err(AssistantError::Field {
            index,
            field: TOOL_CALLS,
        }),
    }
}

fn read_provider_fields(fields: &Object, index: usize) -> Result<Option<&Object>, AssistantError> {
    match fields.get(PROVIDER_SPECIFIC_FIELDS) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(provider)) => Ok(Some(provider)),
        Some(_) => Err(AssistantError::Field {
            index,
            field: PROVIDER_SPECIFIC_FIELDS,
        }),
    }
}

fn block_kind(fields: &Object, index: usize, block: usize) -> Result<String, AssistantError> {
    match fields.get(TYPE) {
        None => Ok(String::new()),
        Some(Json::Text(kind)) => Ok(kind.clone()),
        Some(_) => Err(AssistantError::BlockField {
            index,
            block,
            field: TYPE,
        }),
    }
}

fn list_has_thinking(content: ContentField<'_>) -> bool {
    let ContentField::Blocks(blocks) = content else {
        return false;
    };
    blocks.iter().any(|block| {
        block
            .as_object()
            .and_then(|fields| fields.get(TYPE))
            .and_then(Json::as_str)
            .is_some_and(|kind| kind == THINKING || kind == REDACTED_THINKING)
    })
}

fn has_server_tool_call(calls: Option<&[Json]>) -> bool {
    calls.is_some_and(|declared| {
        declared.iter().any(|call| {
            call.as_object()
                .and_then(|fields| fields.get(ID))
                .and_then(Json::as_str)
                .is_some_and(|id| id.starts_with(SERVER_TOOL_PREFIX))
        })
    })
}

fn convert_tool_calls(parsed: &ParsedAssistant<'_>) -> Result<Vec<Object>, AssistantError> {
    let web_search_results = server_results(parsed.provider, WEB_SEARCH_RESULTS, parsed.index)?;
    let tool_results = server_results(parsed.provider, TOOL_RESULTS, parsed.index)?;
    to_anthropic_tool_invokes(
        parsed.calls.unwrap_or_default(),
        web_search_results,
        tool_results,
    )
    .map_err(AssistantError::ToolInvoke)
}

fn server_results<'input>(
    provider: Option<&'input Object>,
    field: &'static str,
    index: usize,
) -> Result<&'input [Json], AssistantError> {
    match provider.and_then(|fields| fields.get(field)) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::List(results)) => Ok(results),
        Some(_) => Err(AssistantError::Field { index, field }),
    }
}

fn group_tool_invokes(invokes: Vec<Object>) -> (Vec<Vec<Object>>, Vec<Object>) {
    let mut groups: Vec<Vec<Object>> = Vec::new();
    let mut current: Vec<Object> = Vec::new();
    let mut regular: Vec<Object> = Vec::new();
    for block in invokes {
        match block.get(TYPE).and_then(Json::as_str).unwrap_or_default() {
            SERVER_TOOL_USE => {
                if !current.is_empty() {
                    groups.push(take(&mut current));
                }
                current.push(block);
            }
            kind if kind.ends_with(TOOL_RESULT_SUFFIX) => current.push(block),
            TOOL_USE => regular.push(block),
            _ => current.push(block),
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    (groups, regular)
}
