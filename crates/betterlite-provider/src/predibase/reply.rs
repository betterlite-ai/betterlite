use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_openai::chat::choice_finish_reason;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::builtin::Message;
use serde_json::Number;

use super::decode::{
    DecodedReply, NullableString, Sequence, TextReply, Token, TokenLogprob, decode_predibase_reply,
    output_parser,
};
use super::error::PredibaseError;
use super::request::RequestBody;

#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64>;
    async fn count_text(&self, text: &str) -> Option<i64>;
}

pub struct ReplyInput<'request> {
    pub body: &'request [u8],
    pub status: u16,
    pub headers: &'request BTreeMap<String, String>,
    pub request_body: &'request RequestBody,
    pub options: &'request Object,
    pub messages: &'request [Message],
    pub model: &'request str,
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredibaseUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredibaseChoice {
    pub index: u64,
    pub finish_reason: String,
    pub content: Option<String>,
    // the serialized message logprobs of a best_of choice
    pub logprobs: Option<Number>,
    // LiteLLM stores the first choice's logprob sum on a private _logprob attribute
    pub private_logprob: Option<Number>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredibaseResponse {
    pub model: String,
    pub created: i64,
    pub choices: Vec<PredibaseChoice>,
    pub usage: PredibaseUsage,
    pub additional_headers: BTreeMap<String, String>,
}

pub async fn transform_response(
    reply: &ReplyInput<'_>,
    counter: &dyn TokenCounter,
) -> Result<PredibaseResponse, PredibaseError> {
    let text = match decode_predibase_reply(reply.body)? {
        DecodedReply::Text(text) => text,
        DecodedReply::Error(message) => {
            return Err(PredibaseError {
                status_code: reply.status,
                message,
                headers: BTreeMap::new(),
            });
        }
    };
    let mut choices = vec![first_choice(&text)?];
    choices.extend(extra_choices(&text, reply)?);
    let content = choices.first().and_then(|choice| choice.content.as_deref());
    let usage = usage(content, reply.messages, counter).await;
    Ok(PredibaseResponse {
        model: reply.model.to_owned(),
        created: reply.created,
        choices,
        usage,
        additional_headers: additional_headers(reply.headers),
    })
}

fn first_choice(text: &TextReply) -> Result<PredibaseChoice, PredibaseError> {
    let content = (!text.generated_text.is_empty()).then(|| output_parser(&text.generated_text));
    let (finish_reason, private_logprob) = if let Some(details) = text.details.as_ref()
        && let Some(tokens) = details.tokens.as_ref()
    {
        (
            detail_finish_reason(details.finish_reason.as_ref())?,
            Some(sum_logprobs(tokens)?),
        )
    } else {
        (String::from("stop"), None)
    };
    Ok(PredibaseChoice {
        index: 0,
        finish_reason,
        content,
        logprobs: None,
        private_logprob,
    })
}

fn detail_finish_reason(reason: Option<&Option<String>>) -> Result<String, PredibaseError> {
    match reason {
        Some(Some(text)) => {
            let (mapped, _) = choice_finish_reason(Some(text.as_str()));
            Ok(mapped)
        }
        Some(None) => Ok(String::from("stop")),
        None => Err(PredibaseError {
            status_code: 500,
            message: String::from("the predibase reply details have no finish_reason"),
            headers: BTreeMap::new(),
        }),
    }
}

fn extra_choices(
    text: &TextReply,
    reply: &ReplyInput<'_>,
) -> Result<Vec<PredibaseChoice>, PredibaseError> {
    if best_of(reply) <= 1 {
        return Ok(Vec::new());
    }
    let Some(sequences) = text
        .details
        .as_ref()
        .and_then(|details| details.best_of_sequences.as_ref())
    else {
        return Ok(Vec::new());
    };
    (1_u64..)
        .zip(sequences.iter())
        .map(|(index, sequence)| sequence_choice(index, sequence))
        .collect()
}

fn sequence_choice(index: u64, sequence: &Sequence) -> Result<PredibaseChoice, PredibaseError> {
    let (content, logprobs) = if sequence.generated_text.is_empty() {
        (None, None)
    } else {
        (
            Some(output_parser(&sequence.generated_text)),
            Some(sum_logprobs(&sequence.tokens)?),
        )
    };
    let finish_reason = match &sequence.finish_reason {
        NullableString::Text(text) => {
            let (mapped, _) = choice_finish_reason(Some(text.as_str()));
            mapped
        }
        NullableString::Null => String::from("stop"),
    };
    Ok(PredibaseChoice {
        index,
        finish_reason,
        content,
        logprobs,
        private_logprob: None,
    })
}

fn best_of(reply: &ReplyInput<'_>) -> i64 {
    let chosen = reply
        .options
        .get("best_of")
        .filter(|value| !matches!(*value, Json::Null));
    if let Some(value) = chosen {
        return integer_best_of(value);
    }
    reply
        .request_body
        .parameters
        .get("best_of")
        .map_or(0, integer_best_of)
}

fn integer_best_of(value: &Json) -> i64 {
    let Json::Number(number) = value else {
        return 0;
    };
    if let Some(integer) = number.as_i64() {
        return integer;
    }
    number
        .as_u64()
        .map_or(0, |raw| i64::try_from(raw).unwrap_or(i64::MAX))
}

async fn usage(
    content: Option<&str>,
    messages: &[Message],
    counter: &dyn TokenCounter,
) -> PredibaseUsage {
    let prompt_tokens = counter.count_messages(messages).await.unwrap_or(0);
    let completion_tokens = match content {
        Some(text) if !text.is_empty() => counter.count_text(text).await.unwrap_or(0),
        _ => 0,
    };
    PredibaseUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
    }
}

fn additional_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut copied = BTreeMap::new();
    for (name, value) in headers {
        let lowercase = name.to_lowercase();
        if lowercase.starts_with("x-") {
            copied.insert(format!("llm_provider-{lowercase}"), value.clone());
        }
    }
    copied
}

fn sum_logprobs(tokens: &[Token]) -> Result<Number, PredibaseError> {
    logprob_total(tokens).ok_or_else(|| PredibaseError {
        status_code: 500,
        message: String::from("the predibase reply logprob sum is not a JSON number"),
        headers: BTreeMap::new(),
    })
}

fn logprob_total(tokens: &[Token]) -> Option<Number> {
    let numbers = tokens.iter().filter_map(token_logprob);
    if all_integers(tokens) {
        let total = numbers
            .map(integer_logprob)
            .try_fold(0_i128, i128::checked_add)?;
        return Number::from_i128(total);
    }
    let fractions: Vec<f64> = numbers.filter_map(Number::as_f64).collect();
    Number::from_f64(fractions.iter().sum())
}

const fn token_logprob(token: &Token) -> Option<&Number> {
    match &token.logprob {
        TokenLogprob::Number(number) => Some(number),
        TokenLogprob::Null => None,
    }
}

fn integer_logprob(number: &Number) -> i128 {
    match (number.as_i64(), number.as_u64()) {
        (Some(value), _) => i128::from(value),
        (None, Some(value)) => i128::from(value),
        (None, None) => 0,
    }
}

fn all_integers(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .filter_map(token_logprob)
        .all(|number| number.is_i64() || number.is_u64())
}
