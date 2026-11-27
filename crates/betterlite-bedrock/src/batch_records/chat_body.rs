use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::message_content::prompt::to_messages;

use super::error::BatchRecordsError;
use super::families::BatchFamilies;
use crate::batch_embeddings::classify::BatchRecordKind;

const MESSAGES_FIELD: &str = "messages";
const MODEL_FIELD: &str = "model";
const PROMPT_FIELD: &str = "prompt";

// _transform_batch_body_to_chat_body: every non-embedding record becomes a chat body first.
pub async fn to_chat_body(
    body: &Json,
    kind: BatchRecordKind,
    families: &dyn BatchFamilies,
) -> Result<Json, BatchRecordsError> {
    match kind {
        BatchRecordKind::Chat => Ok(body.clone()),
        BatchRecordKind::TextCompletion => text_completion_to_chat(body),
        BatchRecordKind::Responses => {
            families
                .responses_to_chat(body)
                .await
                .map_err(|source| BatchRecordsError::Family {
                    family: "responses",
                    source,
                })
        }
        BatchRecordKind::Embedding => Err(BatchRecordsError::EmbeddingChatBody),
    }
}

// _transform_text_completion_body_to_chat_body: the prompt every Bedrock body lacks becomes messages.
pub fn text_completion_to_chat(body: &Json) -> Result<Json, BatchRecordsError> {
    let Json::Object(fields) = body else {
        return Err(BatchRecordsError::RecordShape);
    };
    let Some(prompt) = fields
        .get(PROMPT_FIELD)
        .filter(|value| **value != Json::Null)
    else {
        return Err(BatchRecordsError::MissingPrompt {
            model: model_text(fields.get(MODEL_FIELD)),
        });
    };
    let prompt_value: PromptJson =
        serde_json::from_value(serde_json::to_value(prompt).map_err(BatchRecordsError::Wire)?)
            .map_err(BatchRecordsError::Wire)?;
    let prompt_messages = to_messages(&prompt_value)?;
    let messages: Vec<Json> = serde_json::from_value(
        serde_json::to_value(prompt_messages).map_err(BatchRecordsError::Wire)?,
    )
    .map_err(BatchRecordsError::Wire)?;
    let mut converted = fields.clone();
    converted.shift_remove(PROMPT_FIELD);
    converted.insert(String::from(MESSAGES_FIELD), Json::List(messages));
    Ok(Json::Object(converted))
}

// `openai_request_body.get("model", "")`: an absent model names nothing, and a non-string one never reaches here.
fn model_text(value: Option<&Json>) -> String {
    match value {
        Some(Json::Text(model)) => model.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    }
}
