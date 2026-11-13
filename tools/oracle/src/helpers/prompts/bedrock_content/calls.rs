use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::bedrock_content::continuation::{
    assistant_continue_message as continue_fields, assistant_message_or_continue,
    insert_assistant_continue, user_message_block_or_continue,
};
use betterlite_prompts::bedrock_content::dedup::{
    TOOL_RESULT_BLOCK, TOOL_USE_ID, deduplicate_content_blocks,
};
use betterlite_prompts::bedrock_content::documents::{
    rename_duplicate_documents, with_text_when_document_only,
};
use betterlite_prompts::bedrock_content::empty_blocks::{
    process_empty_text_blocks, skip_empty_text_blocks,
};
use betterlite_prompts::bedrock_content::error::ContentError;
use betterlite_prompts::bedrock_content::order::sort_assistant_content_blocks;
use serde::Deserialize;

use super::Execute;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeduplicateContent {
    pub(crate) blocks: Vec<Json>,
    pub(crate) block_key: String,
    #[serde(default = "default_id_key")]
    pub(crate) id_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeduplicateToolContent {
    pub(crate) tool_content: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RenameDuplicateDocuments {
    pub(crate) contents: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WithTextWhenDocumentOnly {
    pub(crate) message: Json,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnsureDocumentMessagesHaveText {
    pub(crate) contents: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SortAssistantContent {
    pub(crate) blocks: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InsertAssistantContinue {
    pub(crate) messages: Vec<Json>,
    #[serde(default)]
    pub(crate) assistant_continue_message: Option<Json>,
    #[serde(default)]
    pub(crate) modify_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserMessageBlockOrContinue {
    pub(crate) message: Json,
    #[serde(default)]
    pub(crate) user_continue_message: Option<Json>,
    #[serde(default)]
    pub(crate) modify_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReturnAssistantContinue {
    #[serde(default)]
    pub(crate) assistant_continue_message: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkipEmptyTextBlocks {
    pub(crate) message: Json,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessEmptyTextBlocks {
    pub(crate) message: Json,
    #[serde(default)]
    pub(crate) assistant_continue_message: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssistantMessageOrContinue {
    pub(crate) message: Json,
    #[serde(default)]
    pub(crate) assistant_continue_message: Option<Json>,
    #[serde(default)]
    pub(crate) modify_params: bool,
}

fn default_id_key() -> String {
    TOOL_USE_ID.to_owned()
}

impl Execute for DeduplicateContent {
    fn execute(&self) -> Result<Json, ContentError> {
        deduplicate_content_blocks(&self.blocks, &self.block_key, &self.id_key).map(Json::List)
    }
}

impl Execute for DeduplicateToolContent {
    fn execute(&self) -> Result<Json, ContentError> {
        deduplicate_content_blocks(&self.tool_content, TOOL_RESULT_BLOCK, TOOL_USE_ID)
            .map(Json::List)
    }
}

impl Execute for RenameDuplicateDocuments {
    fn execute(&self) -> Result<Json, ContentError> {
        rename_duplicate_documents(&self.contents).map(Json::List)
    }
}

impl Execute for WithTextWhenDocumentOnly {
    fn execute(&self) -> Result<Json, ContentError> {
        with_text_when_document_only(&self.message)
    }
}

impl Execute for EnsureDocumentMessagesHaveText {
    fn execute(&self) -> Result<Json, ContentError> {
        self.contents
            .iter()
            .map(with_text_when_document_only)
            .collect::<Result<Vec<_>, ContentError>>()
            .map(Json::List)
    }
}

impl Execute for SortAssistantContent {
    fn execute(&self) -> Result<Json, ContentError> {
        sort_assistant_content_blocks(&self.blocks).map(Json::List)
    }
}

impl Execute for InsertAssistantContinue {
    fn execute(&self) -> Result<Json, ContentError> {
        Ok(Json::List(insert_assistant_continue(
            &self.messages,
            self.assistant_continue_message.as_ref(),
            self.modify_params,
        )))
    }
}

impl Execute for UserMessageBlockOrContinue {
    fn execute(&self) -> Result<Json, ContentError> {
        user_message_block_or_continue(
            &self.message,
            self.user_continue_message.as_ref(),
            self.modify_params,
        )
    }
}

impl Execute for ReturnAssistantContinue {
    fn execute(&self) -> Result<Json, ContentError> {
        Ok(Json::Object(continue_fields(
            self.assistant_continue_message.as_ref(),
        )))
    }
}

impl Execute for SkipEmptyTextBlocks {
    fn execute(&self) -> Result<Json, ContentError> {
        skip_empty_text_blocks(&self.message)
    }
}

impl Execute for ProcessEmptyTextBlocks {
    fn execute(&self) -> Result<Json, ContentError> {
        process_empty_text_blocks(&self.message, self.assistant_continue_message.as_ref())
    }
}

impl Execute for AssistantMessageOrContinue {
    fn execute(&self) -> Result<Json, ContentError> {
        assistant_message_or_continue(
            &self.message,
            self.assistant_continue_message.as_ref(),
            self.modify_params,
        )
    }
}
