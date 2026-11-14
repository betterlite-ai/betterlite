use std::borrow::Cow;

use betterlite_models::decisions::Json;

use super::RequestData;
use super::audio::{estimate_audio_content_tokens, strip_audio_content_blocks};
use super::call_family::{CallFamily, call_family};
use super::google::translate_google_request;
use super::input_text::{
    json_list, pretokenized_embedding_tokens, rerank_input_to_text, responses_chat_messages,
    selected_text_chars,
};
use super::output_cap::{
    explicit_output_cap, is_embedding_request, output_candidate_count, output_floor,
};
use super::services::{
    GenAiTranslation, InputTokenCounter, ResponsesInput, TokenCounterRequest, TranslatedChat,
};
use crate::limiter_records::DEFAULT_CHARS_PER_TOKEN;

pub struct TokenEstimateRequest<'request> {
    pub data: &'request RequestData,
    pub call_type: Option<&'request str>,
    pub model: Option<&'request str>,
    pub min_configured_tpm_limit: Option<i64>,
    pub configured_output_tokens: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputOutputTokens {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[must_use]
pub fn input_and_output_tokens(
    translation: &dyn GenAiTranslation,
    request: &TokenEstimateRequest<'_>,
) -> InputOutputTokens {
    let translated = translate_google_request(translation, request.data, request.call_type);
    let (messages, prompt, input) =
        selected_fields(request.data, translated.as_ref(), request.call_type);
    let total_chars = selected_text_chars(messages, prompt, input);
    let estimated_input_tokens = if total_chars > 0 {
        i64::try_from(total_chars)
            .unwrap_or(i64::MAX)
            .div_euclid(DEFAULT_CHARS_PER_TOKEN)
            .max(1)
    } else {
        0
    };
    let explicit_max_tokens = explicit_output_cap(request.data, request.call_type);
    let output_budget = if is_embedding_request(request.data, request.call_type)
        || (explicit_max_tokens.is_none()
            && total_chars == 0
            && request.configured_output_tokens.is_none())
    {
        0
    } else if let Some(explicit) = explicit_max_tokens {
        explicit
    } else if let Some(configured) = request.configured_output_tokens {
        configured
    } else {
        estimated_input_tokens.max(output_floor(
            request.min_configured_tpm_limit,
            request.call_type,
        ))
    };
    InputOutputTokens {
        input_tokens: estimated_input_tokens,
        output_tokens: output_budget
            .saturating_mul(output_candidate_count(request.data, request.call_type)),
    }
}

#[must_use]
pub fn estimate_tokens_for_request(
    translation: &dyn GenAiTranslation,
    request: &TokenEstimateRequest<'_>,
) -> i64 {
    let estimate = input_and_output_tokens(translation, request);
    let total = estimate.input_tokens.saturating_add(estimate.output_tokens);
    tracing::debug!(
        input = estimate.input_tokens,
        max_tokens = estimate.output_tokens,
        total,
        "TPM reservation estimate"
    );
    total
}

#[must_use]
pub async fn estimate_precise_input_tokens(
    translation: &dyn GenAiTranslation,
    responses: &dyn ResponsesInput,
    counter: &dyn InputTokenCounter,
    request: &TokenEstimateRequest<'_>,
) -> i64 {
    let data = request.data;
    let family = call_family(request.call_type);
    let is_responses = matches!(family, CallFamily::Responses);
    let translated = if is_responses {
        None
    } else {
        translate_google_request(translation, data, request.call_type)
    };
    let embedding_input = data
        .get("input")
        .filter(|_| is_embedding_request(data, request.call_type));
    if let Some(pretokenized) = embedding_input.and_then(pretokenized_embedding_tokens) {
        return pretokenized;
    }
    let responses_messages = is_responses.then(|| responses_chat_messages(responses, data));
    let input = counted_input(
        data,
        translated.as_ref(),
        responses_messages,
        request.call_type,
    );
    let audio_tokens = estimate_audio_content_tokens(input.messages.as_deref());
    let stripped = input.messages.as_deref().map(strip_audio_content_blocks);
    if let Some(tokens) = counter_tokens(counter, stripped.as_ref(), &input, request.model).await {
        return tokens.max(0).saturating_add(audio_tokens);
    }
    fallback_tokens(
        translation,
        request,
        family,
        input.text.as_deref(),
        audio_tokens,
    )
}

fn selected_fields<'data>(
    data: &'data RequestData,
    translated: Option<&'data TranslatedChat>,
    call_type: Option<&str>,
) -> (
    Option<&'data Json>,
    Option<&'data Json>,
    Option<&'data Json>,
) {
    let messages = match translated {
        Some(translated) => Some(&translated.messages),
        None => data.get("messages"),
    };
    match call_family(call_type) {
        CallFamily::Responses | CallFamily::Embedding => (None, None, data.get("input")),
        CallFamily::TextCompletion => (None, data.get("prompt"), None),
        CallFamily::Google | CallFamily::Rerank | CallFamily::Other => {
            if call_type.is_some() {
                return (messages, None, None);
            }
            (messages, data.get("prompt"), data.get("input"))
        }
    }
}

struct CountedInput<'request> {
    messages: Option<Cow<'request, Json>>,
    text: Option<Cow<'request, Json>>,
    tools: Option<&'request Json>,
    tool_choice: Option<&'request Json>,
}

fn counted_input<'request>(
    data: &'request RequestData,
    translated: Option<&'request TranslatedChat>,
    responses_messages: Option<Vec<Json>>,
    call_type: Option<&str>,
) -> CountedInput<'request> {
    let family = call_family(call_type);
    let is_embedding = is_embedding_request(data, call_type);
    let messages = if let Some(messages) = responses_messages {
        Some(Cow::Owned(Json::List(messages)))
    } else if let Some(translated) = translated {
        Some(Cow::Borrowed(&translated.messages))
    } else if is_embedding || matches!(family, CallFamily::Rerank | CallFamily::TextCompletion) {
        None
    } else {
        data.get("messages").map(Cow::Borrowed)
    };
    let text = if matches!(family, CallFamily::Responses) || translated.is_some() {
        None
    } else if is_embedding {
        data.get("input").map(Cow::Borrowed)
    } else if matches!(family, CallFamily::Rerank) {
        Some(Cow::Owned(Json::Text(rerank_input_to_text(data))))
    } else if matches!(family, CallFamily::TextCompletion) {
        data.get("prompt").map(Cow::Borrowed)
    } else {
        match data.get("prompt") {
            Some(prompt) if !matches!(prompt, Json::Null) => Some(Cow::Borrowed(prompt)),
            Some(_) | None => data.get("input").map(Cow::Borrowed),
        }
    };
    let (tools, tool_choice) = match translated {
        Some(translated) => (translated.tools.as_ref(), translated.tool_choice.as_ref()),
        None => (data.get("tools"), data.get("tool_choice")),
    };
    CountedInput {
        messages,
        text,
        tools,
        tool_choice,
    }
}

async fn counter_tokens(
    counter: &dyn InputTokenCounter,
    countable: Option<&Json>,
    input: &CountedInput<'_>,
    model: Option<&str>,
) -> Option<i64> {
    let request = TokenCounterRequest {
        model: model.unwrap_or_default(),
        messages: countable.and_then(json_list),
        text: input.text.as_deref(),
        tools: input.tools.and_then(json_list),
        tool_choice: input.tool_choice,
    };
    counter.count_tokens(&request).await
}

fn fallback_tokens(
    translation: &dyn GenAiTranslation,
    request: &TokenEstimateRequest<'_>,
    family: CallFamily,
    text: Option<&Json>,
    audio_tokens: i64,
) -> i64 {
    if matches!(family, CallFamily::Rerank) {
        if let Some(Json::Text(rerank_text)) = text {
            return i64::try_from(rerank_text.chars().count())
                .unwrap_or(i64::MAX)
                .div_euclid(DEFAULT_CHARS_PER_TOKEN)
                .max(0);
        }
    }
    cheap_input_tokens(translation, request).saturating_add(audio_tokens)
}

fn cheap_input_tokens(
    translation: &dyn GenAiTranslation,
    request: &TokenEstimateRequest<'_>,
) -> i64 {
    let cheap = TokenEstimateRequest {
        data: request.data,
        call_type: request.call_type,
        model: None,
        min_configured_tpm_limit: None,
        configured_output_tokens: None,
    };
    input_and_output_tokens(translation, &cheap).input_tokens
}
