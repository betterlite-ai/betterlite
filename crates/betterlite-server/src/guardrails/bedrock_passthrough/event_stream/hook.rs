use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use thiserror::Error;

use super::frames::{parse_event_stream, rebuild_event_stream};
use super::groups::{Group, RedistributeError, group_texts, redistribute};

// Stands in for LiteLLM's ProxyLogging.post_call_success_hook; B8656's ProxyPostCallHook implements it.
pub trait PostCallSuccessHook: Sync {
    type Error: StdError + Send + Sync + 'static;

    fn post_call_success_hook(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        user_api_key_dict: &BTreeMap<String, GuardrailValue>,
        response: GuardrailValue,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Error>> + Send;
}

#[derive(Debug, Error)]
pub enum EventStreamError<E>
where
    E: StdError + Send + Sync + 'static,
{
    #[error("the post call hook failed: {0}")]
    Hook(#[source] E),
    #[error("the returned text could not be split over the stream")]
    Share(#[from] RedistributeError),
}

pub async fn de_anonymize_event_stream<H>(
    body: &[u8],
    hook: &H,
    user_api_key_dict: &BTreeMap<String, GuardrailValue>,
    data: &BTreeMap<String, GuardrailValue>,
) -> Result<Vec<u8>, EventStreamError<H::Error>>
where
    H: PostCallSuccessHook,
{
    let parsed = parse_event_stream(body);
    let groups = group_texts(&parsed.frames);
    let mut content = Vec::new();
    let mut active: Vec<&Group> = Vec::new();
    for group in &groups.groups {
        let joined: String = group.fragments.concat();
        if joined.is_empty() {
            continue;
        }
        content.push(text_block(joined));
        active.push(group);
    }
    if active.is_empty() {
        return Ok(body.to_vec());
    }
    let synthetic = synthetic_response(content);
    let answer = hook
        .post_call_success_hook(data, user_api_key_dict, synthetic)
        .await
        .map_err(EventStreamError::Hook)?;
    if !matches!(answer, GuardrailValue::Object(_)) {
        tracing::debug!(
            "post_call_success_hook returned {}, leaving event stream unmodified",
            value_type_name(&answer)
        );
        return Ok(body.to_vec());
    }
    let Some(returned) = returned_texts(&answer, active.len()) else {
        return Ok(body.to_vec());
    };
    let new_texts = redistribute(&groups, &returned)?;
    Ok(rebuild_event_stream(&parsed, &new_texts))
}

fn text_block(text: String) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([(
        "text".to_owned(),
        GuardrailValue::Text(text),
    )]))
}

fn synthetic_response(content: Vec<GuardrailValue>) -> GuardrailValue {
    let mut message = BTreeMap::new();
    message.insert(
        "role".to_owned(),
        GuardrailValue::Text("assistant".to_owned()),
    );
    message.insert("content".to_owned(), GuardrailValue::Array(content));
    let mut output = BTreeMap::new();
    output.insert("message".to_owned(), GuardrailValue::Object(message));
    let mut response = BTreeMap::new();
    response.insert("output".to_owned(), GuardrailValue::Object(output));
    response.insert(
        "stopReason".to_owned(),
        GuardrailValue::Text("end_turn".to_owned()),
    );
    GuardrailValue::Object(response)
}

fn returned_texts(answer: &GuardrailValue, count: usize) -> Option<Vec<String>> {
    let GuardrailValue::Object(fields) = answer else {
        return None;
    };
    let Some(GuardrailValue::Object(output)) = fields.get("output") else {
        return None;
    };
    let Some(GuardrailValue::Object(message)) = output.get("message") else {
        return None;
    };
    let Some(GuardrailValue::Array(content)) = message.get("content") else {
        return None;
    };
    let mut texts = Vec::with_capacity(count);
    for index in 0..count {
        let Some(GuardrailValue::Object(block)) = content.get(index) else {
            return None;
        };
        let Some(GuardrailValue::Text(text)) = block.get("text") else {
            return None;
        };
        texts.push(text.clone());
    }
    Some(texts)
}

pub(super) const fn value_type_name(value: &GuardrailValue) -> &'static str {
    match value {
        GuardrailValue::Null => "null",
        GuardrailValue::Boolean(_) => "boolean",
        GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_) => "number",
        GuardrailValue::Text(_) => "string",
        GuardrailValue::Array(_) => "list",
        GuardrailValue::Object(_) => "dict",
    }
}
