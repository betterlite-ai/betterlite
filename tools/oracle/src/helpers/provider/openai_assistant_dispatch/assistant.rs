use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_assistant_dispatch::call::{
    AssistantsCall, AssistantsFacade, AssistantsOperation, OpenAiAssistantOperation,
};
use betterlite_provider::openai_assistant_dispatch::reply::AssistantsReply;
use serde_json::{Value, json};

use super::input::{AssistantsOp, Input};
use super::run::options;

// No aget_assistants flag means the case calls async_get_assistants directly instead of the facade.
pub(super) fn assistants(
    input: &Input,
    operation: AssistantsOp,
) -> Result<OpenAiAssistantOperation, Box<HelperError>> {
    let assistant = match operation {
        AssistantsOp::ListAssistants => AssistantsOperation::List {
            facade: input
                .aget_assistants
                .map_or(AssistantsFacade::AsyncHelper, |aget_assistants| {
                    AssistantsFacade::Facade { aget_assistants }
                }),
            params: input.params.clone(),
        },
        AssistantsOp::CreateAssistant => AssistantsOperation::Create {
            data: Box::new(serde_json::from_value(input.data.clone())?),
            async_requested: true,
        },
        AssistantsOp::DeleteAssistant => AssistantsOperation::Delete {
            assistant_id: input.assistant_id.clone(),
            async_requested: true,
        },
    };
    Ok(OpenAiAssistantOperation::Assistants(AssistantsCall {
        options: options(input),
        operation: assistant,
    }))
}

pub(super) fn assistant_view(reply: &AssistantsReply) -> Value {
    match reply {
        AssistantsReply::Page(page) => {
            let ids: Vec<&str> = page.data.iter().map(|item| item.id.as_str()).collect();
            json!({"ids": ids, "has_more": page.has_more})
        }
        AssistantsReply::Assistant(assistant) => {
            json!({"id": assistant.id, "object": assistant.object})
        }
        AssistantsReply::Deleted(deleted) => json!({
            "id": deleted.id,
            "deleted": deleted.deleted,
            "object": deleted.object,
        }),
    }
}
