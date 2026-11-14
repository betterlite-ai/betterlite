use betterlite_openai::chat::response::{ChatResponse, ChatUsage};
use betterlite_prompts::builtin::Message;
use betterlite_tokenizer::count::default_encoding::DefaultEncoding;
use betterlite_tokenizer::engine::Engine;

use crate::completion::error::{CompletionError, anthropic_text_error};
use crate::completion::prompt::{PromptSource, prompt_from_messages};
use crate::completion::reply::{LegacyReply, decode_legacy_anthropic_reply};

pub struct ResponseInput<'input> {
    pub model: &'input str,
    pub messages: &'input [Message],
    pub status_code: u16,
    pub body: &'input [u8],
}

// the explicit encoding wins, and litellm.encoding answers when the caller passes none
pub async fn resolve_tokenizer(
    encoding: Option<Engine>,
    default_encoding: &DefaultEncoding,
) -> Result<Engine, CompletionError> {
    if let Some(engine) = encoding {
        return Ok(engine);
    }
    default_encoding
        .get()
        .await
        .map_err(|source| CompletionError::DefaultEncoding {
            source: Box::new(source),
        })
}

fn count_tokens(tokenizer: &Engine, text: &str) -> Result<u64, CompletionError> {
    let tokens = tokenizer
        .encode(text)
        .map_err(|source| CompletionError::Tokenize {
            source: Box::new(source),
        })?;
    u64::try_from(tokens.len()).map_err(|_source| CompletionError::Usage)
}

pub async fn transform_response(
    source: &PromptSource<'_>,
    input: &ResponseInput<'_>,
    mut model_response: ChatResponse,
    tokenizer: &Engine,
) -> Result<ChatResponse, CompletionError> {
    let reply = decode_legacy_anthropic_reply(input.status_code, input.body)?;
    // the prompt is rebuilt here for the usage, not read back from the request
    let prompt = prompt_from_messages(source, input.model, input.messages).await?;
    let (completion, stop_reason) = match reply {
        LegacyReply::Error { message } => {
            return Err(CompletionError::Upstream(Box::new(anthropic_text_error(
                input.status_code,
                message,
            ))));
        }
        LegacyReply::Completion {
            completion,
            stop_reason,
        } => (completion, stop_reason),
    };
    let choice = model_response
        .choices
        .first_mut()
        .ok_or(CompletionError::NoChoices)?;
    if !completion.is_empty() {
        choice.message.content = Some(completion);
    }
    choice.finish_reason = stop_reason;
    let prompt_tokens = count_tokens(tokenizer, &prompt)?;
    // an unset content counts as no tokens where tiktoken raises on None
    let completion_tokens = count_tokens(
        tokenizer,
        choice.message.content.as_deref().unwrap_or_default(),
    )?;
    model_response.created = u64::try_from(source.now.timestamp()).unwrap_or_default();
    input.model.clone_into(&mut model_response.model);
    model_response.usage = ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens
            .checked_add(completion_tokens)
            .ok_or(CompletionError::Usage)?,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    };
    Ok(model_response)
}
