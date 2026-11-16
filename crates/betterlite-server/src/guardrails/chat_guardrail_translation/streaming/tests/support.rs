use std::future::Future;
use std::sync::Mutex;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::passthrough_guardrail::wire::parse_request_data;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};

use crate::guardrails::chat_guardrail_translation::extraction::accumulate_string_content::accumulate_by_choice_index;
use crate::guardrails::chat_guardrail_translation::rewrites::ended_stream_gather::EndedStream;
use crate::guardrails::chat_guardrail_translation::rewrites::error::RewriteError;
use crate::guardrails::chat_guardrail_translation::rewrites::rebuild_ended_stream::ChunkAssembler;
use crate::guardrails::chat_guardrail_translation::rewrites::response::{
    ChatResponse, MessageContent, ResponseChoice, ResponseMessage,
};

pub(crate) const TWO_CHOICES: &str = r#"[
    {"model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hel"}}]},
    {"choices": [{"index": 1, "delta": {"content": "Hi"}}]},
    {"choices": [{"index": 0, "delta": {"content": "lo"}}]}]"#;

pub(crate) const ENDED: &str = r#"[
    {"id": "c1", "model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hel"}}]},
    {"id": "c1", "model": "gpt-4o", "choices": [{"index": 0, "delta": {"content": "lo"}}]},
    {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}]"#;

pub(crate) type Reply = Box<dyn Fn(GuardrailApiInputs) -> GuardrailApiInputs + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: Object,
}

pub(crate) struct FakeGuardrail {
    reply: Reply,
    failure: Option<GuardrailFailure>,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeGuardrail {
    pub(crate) fn answering(reply: Reply) -> Self {
        Self {
            reply,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn passing_through() -> Self {
        Self::answering(Box::new(|inputs| inputs))
    }

    pub(crate) fn upper_casing() -> Self {
        Self::answering(Box::new(|inputs| GuardrailApiInputs {
            texts: inputs
                .texts
                .map(|texts| texts.iter().map(|text| text.to_uppercase()).collect()),
            ..inputs
        }))
    }

    pub(crate) fn failing(failure: GuardrailFailure) -> Self {
        Self {
            reply: Box::new(|inputs| inputs),
            failure: Some(failure),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Recorded> {
        self.calls.lock().unwrap().clone()
    }
}

impl GuardrailContract for FakeGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut Object,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        self.calls.lock().unwrap().push(Recorded {
            inputs: inputs.clone(),
            input_type,
            request_data: request_data.clone(),
        });
        let outcome = self
            .failure
            .as_ref()
            .map_or_else(|| Ok((self.reply)(inputs)), |failure| Err(failure.clone()));
        async move { outcome }
    }
}

// Answers each rebuild with one choice holding the narrowed stream's text, as the real builder does for one choice index.
pub(crate) struct ConcatenatingAssembler;

impl ChunkAssembler for ConcatenatingAssembler {
    fn assemble(
        &self,
        chunks: &[Object],
    ) -> impl Future<Output = Result<ChatResponse, RewriteError>> + Send {
        let choices = accumulate_by_choice_index(chunks)
            .into_iter()
            .map(|(index, text)| ResponseChoice {
                index,
                message: ResponseMessage {
                    content: Some(MessageContent::Text(text)),
                    tool_calls: None,
                },
            })
            .collect();
        async move {
            Ok(ChatResponse {
                model: Some("gpt-4o".to_owned()),
                choices,
            })
        }
    }
}

pub(crate) fn object(source: &str) -> Object {
    parse_request_data(source).unwrap()
}

pub(crate) fn chunks(source: &str) -> Vec<Object> {
    let wrapped = object(&format!(r#"{{"items": {source}}}"#));
    let Some(GuardrailValue::Array(items)) = wrapped.get("items") else {
        panic!("expected a JSON array");
    };
    items
        .iter()
        .map(|item| match item {
            GuardrailValue::Object(fields) => fields.clone(),
            other @ (GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_)) => panic!("expected an object, got {other:?}"),
        })
        .collect()
}

pub(crate) fn stream_items(source: &str) -> Vec<StreamItem> {
    chunks(source)
        .into_iter()
        .map(|chunk| StreamItem::Json(GuardrailValue::Object(chunk)))
        .collect()
}

pub(crate) fn principal() -> Principal {
    serde_json::from_str(
        r#"{"principal_type": "service_account", "subject": "tester", "auth_method": "api_key",
            "user": {"id": "user-1", "email": "user@example.com"}}"#,
    )
    .unwrap()
}

pub(crate) fn ended(principal: &Principal, deliver_rewrites: bool) -> EndedStream<'_> {
    EndedStream {
        principal,
        guardrail_name: "pii",
        deliver_rewrites,
    }
}
