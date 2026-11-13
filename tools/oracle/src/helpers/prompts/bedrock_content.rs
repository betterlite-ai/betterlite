use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::bedrock_content::error::ContentError;
use serde::Deserialize;
use serde::Serialize;

use crate::helpers::{HelperError, HelperResult};

mod calls;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    DeduplicateContent(calls::DeduplicateContent),
    DeduplicateToolContent(calls::DeduplicateToolContent),
    RenameDuplicateDocuments(calls::RenameDuplicateDocuments),
    WithTextWhenDocumentOnly(calls::WithTextWhenDocumentOnly),
    EnsureDocumentMessagesHaveText(calls::EnsureDocumentMessagesHaveText),
    SortAssistantContent(calls::SortAssistantContent),
    InsertAssistantContinue(calls::InsertAssistantContinue),
    UserMessageBlockOrContinue(calls::UserMessageBlockOrContinue),
    ReturnAssistantContinue(calls::ReturnAssistantContinue),
    SkipEmptyTextBlocks(calls::SkipEmptyTextBlocks),
    ProcessEmptyTextBlocks(calls::ProcessEmptyTextBlocks),
    AssistantMessageOrContinue(calls::AssistantMessageOrContinue),
}

trait Execute {
    fn execute(&self) -> Result<Json, ContentError>;
}

impl Execute for Input {
    fn execute(&self) -> Result<Json, ContentError> {
        match self {
            Self::DeduplicateContent(call) => call.execute(),
            Self::DeduplicateToolContent(call) => call.execute(),
            Self::RenameDuplicateDocuments(call) => call.execute(),
            Self::WithTextWhenDocumentOnly(call) => call.execute(),
            Self::EnsureDocumentMessagesHaveText(call) => call.execute(),
            Self::SortAssistantContent(call) => call.execute(),
            Self::InsertAssistantContinue(call) => call.execute(),
            Self::UserMessageBlockOrContinue(call) => call.execute(),
            Self::ReturnAssistantContinue(call) => call.execute(),
            Self::SkipEmptyTextBlocks(call) => call.execute(),
            Self::ProcessEmptyTextBlocks(call) => call.execute(),
            Self::AssistantMessageOrContinue(call) => call.execute(),
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        ok_value(
            input
                .execute()
                .map_err(|failure| content_failure(&failure))?,
        )
    }
}

fn content_failure(failure: &ContentError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BedrockContent".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: failure.to_string(),
    })
}

fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: Serialize,
{
    serde_json::to_value(payload).map_err(|failure| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    })
}
