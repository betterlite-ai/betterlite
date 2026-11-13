use std::error::Error as StdError;

use crate::anthropic_invokes::cache_control::add_cache_control_to_content;
use crate::anthropic_invokes::documents::process_openai_file_message;
use crate::anthropic_results::images::create_anthropic_image_param;
use crate::anthropic_results::tool_result::{FileBlocks, to_anthropic_tool_result};
use crate::arguments::json::{Json, Object};
use crate::media::inline::RemoteMediaFetcher;
use crate::media::limits::MAX_MESSAGE_PARTS;
use crate::message_content::Message;

use super::error::UserBlocksError;

const CONTENT: &str = "content";
const DOCUMENT: &str = "document";
const FILE: &str = "file";
const FUNCTION: &str = "function";
const IMAGE_URL: &str = "image_url";
const ROLE: &str = "role";
const TEXT: &str = "text";
const TOOL: &str = "tool";
const TYPE: &str = "type";
const USER: &str = "user";

pub struct UserBlockBuilder<'deps, Fetcher, Blocks, Ids> {
    pub force_base64: bool,
    pub fetcher: &'deps Fetcher,
    pub blocks: &'deps Blocks,
    pub new_id: &'deps Ids,
}

#[derive(Debug)]
pub struct UserRun {
    pub blocks: Vec<Object>,
    pub next: usize,
}

impl UserRun {
    // LiteLLM appends a merged user turn only when the run produced at least one block
    #[must_use]
    pub fn message(&self) -> Option<Json> {
        if self.blocks.is_empty() {
            return None;
        }
        let content = self
            .blocks
            .iter()
            .map(|block| Json::Object(block.clone()))
            .collect();
        Some(Json::Object(Object::from_iter([
            (ROLE.to_owned(), Json::Text(USER.to_owned())),
            (CONTENT.to_owned(), Json::List(content)),
        ])))
    }
}

impl<Fetcher, Blocks, Ids> UserBlockBuilder<'_, Fetcher, Blocks, Ids>
where
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    pub async fn convert(
        &self,
        messages: &[Json],
        start: usize,
    ) -> Result<UserRun, UserBlocksError<Blocks::Failure>> {
        let mut blocks = Vec::new();
        let mut index = start;
        while let Some(message) = messages.get(index) {
            let Some(fields) = message.as_object() else {
                return Err(UserBlocksError::MessageNotObject { index });
            };
            let Some(role) = fields.get(ROLE).and_then(Json::as_str) else {
                return Err(UserBlocksError::MissingRole { index });
            };
            match role {
                USER => blocks.extend(self.user_blocks(fields, index).await?),
                TOOL | FUNCTION => blocks.push(self.tool_result(message, index).await?),
                _ => break,
            }
            index = index.saturating_add(1);
        }
        Ok(UserRun {
            blocks,
            next: index,
        })
    }

    async fn user_blocks(
        &self,
        fields: &Object,
        index: usize,
    ) -> Result<Vec<Object>, UserBlocksError<Blocks::Failure>> {
        let Some(content) = fields.get(CONTENT) else {
            return Ok(Vec::new());
        };
        match content {
            Json::List(parts) => self.parts_blocks(parts, index).await,
            Json::Text(text) => Ok(vec![text_element(text, fields)]),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Ok(Vec::new()),
        }
    }

    async fn parts_blocks(
        &self,
        parts: &[Json],
        index: usize,
    ) -> Result<Vec<Object>, UserBlocksError<Blocks::Failure>> {
        if parts.len() > MAX_MESSAGE_PARTS {
            return Err(UserBlocksError::TooManyParts {
                limit: MAX_MESSAGE_PARTS,
            });
        }
        let mut blocks = Vec::with_capacity(parts.len());
        for (offset, part) in parts.iter().enumerate() {
            let Some(part_fields) = part.as_object() else {
                return Err(UserBlocksError::BlockNotObject {
                    message: index,
                    block: offset,
                });
            };
            if let Some(element) = self.part_block(part_fields, index, offset).await? {
                blocks.push(element);
            }
        }
        Ok(blocks)
    }

    async fn part_block(
        &self,
        part: &Object,
        message: usize,
        block: usize,
    ) -> Result<Option<Object>, UserBlocksError<Blocks::Failure>> {
        let element = match part.get(TYPE).and_then(Json::as_str).unwrap_or_default() {
            IMAGE_URL => {
                let Some(image_url) = part.get(IMAGE_URL) else {
                    return Err(UserBlocksError::Field {
                        message,
                        block,
                        field: IMAGE_URL,
                    });
                };
                add_cache_control_to_content(
                    create_anthropic_image_param(image_url, None, self.force_base64, self.fetcher)
                        .await?,
                    part,
                )
            }
            TEXT => text_block(part, message, block)?,
            DOCUMENT => add_cache_control_to_content(part.clone(), part),
            FILE => self.file_block(part).await?,
            _ => return Ok(None),
        };
        Ok(Some(element))
    }

    async fn file_block(&self, part: &Object) -> Result<Object, UserBlocksError<Blocks::Failure>> {
        Ok(add_cache_control_to_content(
            process_openai_file_message(part, self.fetcher).await?,
            part,
        ))
    }

    async fn tool_result(
        &self,
        source: &Json,
        index: usize,
    ) -> Result<Object, UserBlocksError<Blocks::Failure>> {
        let value =
            serde_json::to_value(source).map_err(|failure| UserBlocksError::MessageDecode {
                index,
                source: failure,
            })?;
        let message = serde_json::from_value::<Message>(value).map_err(|failure| {
            UserBlocksError::MessageDecode {
                index,
                source: failure,
            }
        })?;
        to_anthropic_tool_result(
            &message,
            self.force_base64,
            self.fetcher,
            self.blocks,
            self.new_id,
        )
        .await
        .map_err(UserBlocksError::ToolResult)
    }
}

fn text_block<FileFailure>(
    part: &Object,
    message: usize,
    block: usize,
) -> Result<Object, UserBlocksError<FileFailure>>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    let Some(Json::Text(text)) = part.get(TEXT) else {
        return Err(UserBlocksError::Field {
            message,
            block,
            field: TEXT,
        });
    };
    Ok(text_element(text, part))
}

fn text_element(text: &str, original: &Object) -> Object {
    let element = Object::from_iter([
        (TYPE.to_owned(), Json::Text(TEXT.to_owned())),
        (TEXT.to_owned(), Json::Text(text.to_owned())),
    ]);
    add_cache_control_to_content(element, original)
}
