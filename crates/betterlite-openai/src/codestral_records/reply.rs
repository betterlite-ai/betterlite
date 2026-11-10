use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codestral_records::error::{NativeError, TextCompletionCodestralError};
use crate::codestral_records::{RecordSource, record_text};
use crate::text_completion_policy::reply::mapped_finish_reason;

pub struct PostCall<'request> {
    pub input: &'request [Value],
    pub api_key: &'request str,
    pub original_response: &'request str,
    pub request_data: &'request Value,
}

pub trait PostCallLogger {
    fn post_call(&mut self, call: PostCall<'_>);
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CodestralUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
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

#[derive(Deserialize)]
struct ReplyWire {
    id: Option<String>,
    created: Option<i64>,
    model: Option<String>,
    usage: Option<CodestralUsage>,
    #[serde(default)]
    choices: Vec<ChoiceWire>,
}

#[derive(Deserialize)]
struct ChoiceWire {
    index: i64,
    message: Option<MessageWire>,
    finish_reason: Option<String>,
    logprobs: Option<Value>,
}

#[derive(Deserialize)]
struct MessageWire {
    content: Option<String>,
}

// LiteLLM logs the raw answer before it looks at the status, so a rejected reply still reaches post_call
pub fn process_response(
    status_code: u16,
    call: PostCall<'_>,
    logger: &mut dyn PostCallLogger,
    source: &RecordSource,
) -> Result<CodestralReply, TextCompletionCodestralError> {
    let body = call.original_response;
    logger.post_call(call);
    decode_codestral_reply(status_code, body.as_bytes(), source)
}

pub fn decode_codestral_reply(
    status_code: u16,
    body: &[u8],
    source: &RecordSource,
) -> Result<CodestralReply, TextCompletionCodestralError> {
    let text = record_text(body)?;
    let native = |code| {
        TextCompletionCodestralError::Native(Box::new(NativeError::new(
            code,
            text.clone(),
            None,
            None,
        )))
    };
    if status_code != 200 {
        return Err(native(status_code));
    }
    let wire: ReplyWire = serde_json::from_str(&text).map_err(|_invalid| native(422))?;
    Ok(CodestralReply {
        id: wire.id.unwrap_or_else(|| format!("chatcmpl-{}", source.id)),
        // TextCompletionResponse.__init__ overwrites whatever object the provider sent
        object: "text_completion".to_owned(),
        created: wire.created.unwrap_or(source.created),
        model: wire.model,
        // TextChoices maps a non-blank reason and keeps a blank one as None
        choices: wire
            .choices
            .into_iter()
            .map(|choice| CodestralReplyChoice {
                index: choice.index,
                text: choice.message.and_then(|message| message.content),
                finish_reason: choice
                    .finish_reason
                    .filter(|reason| !reason.is_empty())
                    .map(|reason| mapped_finish_reason(&reason).to_owned()),
                logprobs: choice.logprobs,
            })
            .collect(),
        usage: wire.usage.unwrap_or_default(),
    })
}
