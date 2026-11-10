use std::str;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codestral_records::error::{NativeError, TextCompletionCodestralError};
use crate::codestral_records::{MAX_RECORD_BYTES, RecordSource};
use crate::text_completion_policy::reply::mapped_finish_reason;

pub struct CodestralHttpResponse<'body> {
    pub status_code: u16,
    pub text: &'body str,
}

pub struct PostCall<'request> {
    pub input: &'request [Value],
    pub api_key: &'request str,
    pub original_response: &'request str,
    pub request_data: &'request Value,
}

pub trait PostCallLogger {
    fn post_call(&mut self, call: PostCall<'_>);
}

pub struct ReplyInput<'body> {
    pub response: CodestralHttpResponse<'body>,
    pub messages: &'body [Value],
    pub api_key: &'body str,
    pub request_data: &'body Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CodestralUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodestralChoiceMessage {
    pub content: Option<String>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodestralChoice {
    pub index: i64,
    pub message: Option<CodestralChoiceMessage>,
    pub finish_reason: Option<String>,
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodestralCompletionResponse {
    pub id: Option<String>,
    pub created: Option<i64>,
    pub model: Option<String>,
    pub object: Option<String>,
    pub usage: Option<CodestralUsage>,
    #[serde(default)]
    pub choices: Vec<CodestralChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodestralReplyChoice {
    pub index: i64,
    pub text: Option<String>,
    pub finish_reason: Option<String>,
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodestralReply {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: Option<String>,
    pub choices: Vec<CodestralReplyChoice>,
    pub usage: CodestralUsage,
}

// LiteLLM logs the raw answer before it looks at the status, so a rejected reply still reaches post_call
pub fn process_response(
    input: &ReplyInput<'_>,
    logger: &mut dyn PostCallLogger,
    source: &RecordSource,
) -> Result<CodestralReply, TextCompletionCodestralError> {
    logger.post_call(PostCall {
        input: input.messages,
        api_key: input.api_key,
        original_response: input.response.text,
        request_data: input.request_data,
    });
    decode_codestral_reply(
        input.response.status_code,
        input.response.text.as_bytes(),
        source,
    )
}

pub fn decode_codestral_reply(
    status_code: u16,
    body: &[u8],
    source: &RecordSource,
) -> Result<CodestralReply, TextCompletionCodestralError> {
    if body.len() > MAX_RECORD_BYTES {
        return Err(TextCompletionCodestralError::TooLarge {
            size: body.len(),
            limit: MAX_RECORD_BYTES,
        });
    }
    let text = str::from_utf8(body).map_err(TextCompletionCodestralError::BodyNotUtf8)?;
    if status_code != 200 {
        return Err(TextCompletionCodestralError::Native(Box::new(
            NativeError::new(status_code, text.to_owned(), None, None),
        )));
    }
    let response = match serde_json::from_str::<CodestralCompletionResponse>(text) {
        Ok(response) => response,
        Err(_invalid) => {
            return Err(TextCompletionCodestralError::Native(Box::new(
                NativeError::new(422, text.to_owned(), None, None),
            )));
        }
    };
    Ok(reply_from(response, source))
}

#[must_use]
fn reply_from(response: CodestralCompletionResponse, source: &RecordSource) -> CodestralReply {
    let id = response
        .id
        .unwrap_or_else(|| format!("chatcmpl-{}", source.id));
    let choices = response.choices.into_iter().map(reply_choice).collect();
    CodestralReply {
        id,
        // TextCompletionResponse.__init__ overwrites whatever object the provider sent
        object: "text_completion".to_owned(),
        created: response.created.unwrap_or(source.created),
        model: response.model,
        choices,
        usage: response.usage.unwrap_or_default(),
    }
}

#[must_use]
fn reply_choice(choice: CodestralChoice) -> CodestralReplyChoice {
    // TextChoices maps a non-blank reason and keeps a blank one as None
    let finish_reason = choice
        .finish_reason
        .filter(|reason| !reason.is_empty())
        .map(|reason| mapped_finish_reason(&reason).to_owned());
    CodestralReplyChoice {
        index: choice.index,
        text: choice.message.and_then(|message| message.content),
        finish_reason,
        logprobs: choice.logprobs,
    }
}
