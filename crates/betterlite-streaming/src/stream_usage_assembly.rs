use std::borrow::Cow;

use async_trait::async_trait;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::structured::Payload;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TextInput, TokenRequest};
use thiserror::Error;

pub mod accumulate;
pub mod details;
mod record;
mod tests;
pub mod wire;

use accumulate::calculate_usage_per_chunk;
use details::apply_grounding_request_counts;
use wire::{ReasoningChoice, Usage};

#[derive(Debug, Error)]
pub enum UsageAssemblyError {
    #[error("the completion token count failed")]
    CompletionCount(#[from] CountError),
    #[error("the server tool use record has an invalid {field} field")]
    ServerToolUse { field: String },
    #[error("the token count does not fit in a signed 64-bit integer")]
    Overflow,
}

#[async_trait]
pub trait UsageTokenCounter: Send + Sync {
    async fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError>;
    async fn count_text(&self, text: &str) -> Result<i64, CountError>;
    async fn count_messages(
        &self,
        model: &str,
        messages: Option<&[Message]>,
    ) -> Result<i64, CountError>;
}

pub struct UsageRequest<'request> {
    pub chunks: &'request [wire::Chunk],
    pub model: &'request str,
    pub completion_output: &'request str,
    pub messages: Option<&'request [Message]>,
    pub reasoning_tokens: Option<i64>,
    pub count_prompt_tokens: Option<&'request (dyn Fn() -> Result<i64, CountError> + Send + Sync)>,
}

pub async fn calculate_usage<L, C>(
    counter: &dyn UsageTokenCounter,
    hooks: &VerboseHooks<L, C>,
    request: &UsageRequest<'_>,
) -> Result<Usage, UsageAssemblyError>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let per_chunk = calculate_usage_per_chunk(request.chunks)?;
    let prompt_tokens = match per_chunk.prompt_tokens {
        Some(count) => count,
        None => prompt_tokens_fallback(counter, hooks, request).await,
    };
    let completion_tokens = match per_chunk.completion_tokens {
        Some(count) => count,
        None => completion_tokens_fallback(counter, request).await?,
    };
    let mut usage = Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens
            .checked_add(completion_tokens)
            .ok_or(UsageAssemblyError::Overflow)?,
        cache_creation_input_tokens: per_chunk.cache_creation_input_tokens,
        cache_read_input_tokens: per_chunk.cache_read_input_tokens,
        completion_tokens_details: per_chunk.completion_tokens_details,
        prompt_tokens_details: per_chunk.prompt_tokens_details,
        server_tool_use: per_chunk.server_tool_use,
        cost: per_chunk.cost,
        inference_geo: per_chunk.inference_geo,
        speed: per_chunk.speed,
    };
    apply_reasoning_details(&mut usage, request.reasoning_tokens);
    usage.prompt_tokens_details = apply_grounding_request_counts(
        usage.prompt_tokens_details.as_ref(),
        per_chunk.web_search_requests,
        per_chunk.google_maps_grounding_requests,
    )
    .map(Cow::into_owned);
    apply_reported_cache_counts(&mut usage);
    Ok(usage)
}

// LiteLLM swallows a prompt count failure so a finished stream still returns a usage record.
async fn prompt_tokens_fallback<L, C>(
    counter: &dyn UsageTokenCounter,
    hooks: &VerboseHooks<L, C>,
    request: &UsageRequest<'_>,
) -> i64
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let tokens = match request.count_prompt_tokens {
        Some(callback) => callback(),
        None => {
            counter
                .count_messages(request.model, request.messages)
                .await
        }
    };
    match tokens {
        Ok(count) => count,
        Err(_count_error) => {
            // litellm.utils.print_verbose defaults to log_level DEBUG and logger_only false.
            let _printed = hooks.utils_print_verbose(
                Payload::String("token_counter failed, assuming prompt tokens is 0".to_owned()),
                "DEBUG",
                &Payload::Bool(false),
            );
            0
        }
    }
}

async fn completion_tokens_fallback(
    counter: &dyn UsageTokenCounter,
    request: &UsageRequest<'_>,
) -> Result<i64, UsageAssemblyError> {
    let tokens = counter
        .count_response_text(request.model, request.completion_output)
        .await?;
    tokens
        .checked_add(request.reasoning_tokens.unwrap_or(0))
        .ok_or(UsageAssemblyError::Overflow)
}

// The capped reasoning count fills a details record, and a missing text count is the completion count minus it.
fn apply_reasoning_details(usage: &mut Usage, reasoning_tokens: Option<i64>) {
    let Some(reasoning) = reasoning_tokens else {
        return;
    };
    let capped = reasoning.max(0).min(usage.completion_tokens);
    let text = usage.completion_tokens.saturating_sub(capped);
    let details = usage.completion_tokens_details.get_or_insert_default();
    if details.reasoning_tokens.is_none() {
        details.reasoning_tokens = Some(capped);
        details.text_tokens = details.text_tokens.or(Some(text));
    }
}

// The final Usage(**dump) rebuild feeds the top-level cache counts back into the prompt details wrapper.
fn apply_reported_cache_counts(usage: &mut Usage) {
    if let Some(read) = usage.cache_read_input_tokens {
        usage
            .prompt_tokens_details
            .get_or_insert_default()
            .cached_tokens = Some(read);
    }
    if let Some(creation) = usage.cache_creation_input_tokens {
        let details = usage.prompt_tokens_details.get_or_insert_default();
        details.cache_write_tokens = Some(creation);
        details.cache_creation_tokens = Some(creation);
    }
}

pub async fn count_reasoning_tokens(
    counter: &dyn UsageTokenCounter,
    choices: &[ReasoningChoice],
) -> Result<Option<i64>, UsageAssemblyError> {
    let mut total: Option<i64> = None;
    let texts = choices
        .iter()
        .filter_map(|choice| choice.message.reasoning_content.as_deref())
        .filter(|text| !text.is_empty());
    for text in texts {
        let tokens = counter.count_text(text).await?;
        total = Some(
            total
                .unwrap_or(0)
                .checked_add(tokens)
                .ok_or(UsageAssemblyError::Overflow)?,
        );
    }
    Ok(total)
}

#[async_trait]
impl UsageTokenCounter for Counter {
    async fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError> {
        response_text_tokens(self, Some(model), text).await
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        response_text_tokens(self, None, text).await
    }

    async fn count_messages(
        &self,
        model: &str,
        messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        let request = TokenRequest {
            model,
            custom_tokenizer: None,
            text: None,
            messages,
            count_response_tokens: Some(false),
            tools: None,
            tool_choice: None,
        };
        let total = self.count(&request).await?;
        i64::try_from(total).map_err(|_conversion| CountError::Overflow)
    }
}

async fn response_text_tokens(
    counter: &Counter,
    model: Option<&str>,
    text: &str,
) -> Result<i64, CountError> {
    let input = TextInput::One(text.to_owned());
    let request = TokenRequest {
        model: model.unwrap_or_default(),
        custom_tokenizer: None,
        text: Some(&input),
        messages: None,
        count_response_tokens: Some(true),
        tools: None,
        tool_choice: None,
    };
    let total = counter.count(&request).await?;
    i64::try_from(total).map_err(|_conversion| CountError::Overflow)
}
