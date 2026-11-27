#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    GlobalSkipFlags, GuardrailSkipFlags,
};
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use crate::guardrails::chat_guardrail_translation::rewrites::error::RewriteError;
use crate::guardrails::chat_guardrail_translation::rewrites::rebuild_ended_stream::ChunkAssembler;
use crate::guardrails::chat_guardrail_translation::rewrites::response::ChatResponse;
use crate::guardrails::responses_translation::block_events::BlockStamp;
use crate::guardrails::unified_chat::GuardrailToApply;
use crate::guardrails::unified_chat::stamps::StampSource;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecordedCall {
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: BTreeMap<String, GuardrailValue>,
}

#[derive(Default)]
struct FakeState {
    reply: Option<GuardrailApiInputs>,
    failure: Option<GuardrailFailure>,
    calls: Vec<RecordedCall>,
}

pub(crate) struct FakeGuardrail {
    state: Mutex<FakeState>,
}

impl FakeGuardrail {
    fn with(state: FakeState) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }

    pub(crate) fn replying(reply: GuardrailApiInputs) -> Self {
        Self::with(FakeState {
            reply: Some(reply),
            failure: None,
            calls: Vec::new(),
        })
    }

    pub(crate) fn failing(failure: GuardrailFailure) -> Self {
        Self::with(FakeState {
            reply: None,
            failure: Some(failure),
            calls: Vec::new(),
        })
    }

    pub(crate) fn calls(&self) -> Vec<RecordedCall> {
        self.state.lock().unwrap().calls.clone()
    }
}

impl GuardrailContract for FakeGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        let (reply, failure) = {
            let mut state = self.state.lock().unwrap();
            state.calls.push(RecordedCall {
                inputs: inputs.clone(),
                input_type,
                request_data: request_data.clone(),
            });
            (state.reply.clone(), state.failure.clone())
        };
        async move {
            if let Some(denial) = failure {
                return Err(denial);
            }
            Ok(reply.unwrap_or(inputs))
        }
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        Some(GuardrailValue::Text(String::from("model")))
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(vec![GuardrailEventHooks::PostCall])
    }

    fn supports_tool_result_scan(&self) -> bool {
        false
    }

    fn messages_cover_full_request(&self) -> bool {
        true
    }
}

pub(crate) fn principal() -> Principal {
    serde_json::from_str(r#"{"principal_type":"human","subject":"sub","auth_method":"api_key"}"#)
        .unwrap()
}

pub(crate) fn context(
    guardrail: &Arc<FakeGuardrail>,
    guardrail_name: Option<&str>,
) -> GuardrailToApply<FakeGuardrail> {
    GuardrailToApply {
        guardrail: Arc::clone(guardrail),
        guardrail_name: guardrail_name.map(str::to_owned),
        principal: principal(),
        skip_flags: GuardrailSkipFlags::default(),
        global_flags: GlobalSkipFlags::default(),
    }
}

pub(crate) fn texts_reply(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

pub(crate) fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn object(json: &str) -> Object {
    let GuardrailValue::Object(fields) = value(json) else {
        panic!("expected a JSON object");
    };
    fields
}

pub(crate) fn items(json: &str) -> Vec<StreamItem> {
    match value(json) {
        GuardrailValue::Array(values) => values.into_iter().map(StreamItem::Json).collect(),
        other @ (GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Object(_)) => panic!("expected a JSON array, got {other:?}"),
    }
}

pub(crate) fn nested<'item>(
    item: &'item GuardrailValue,
    path: &[&str],
) -> Option<&'item GuardrailValue> {
    let mut current = item;
    for name in path {
        let GuardrailValue::Object(fields) = current else {
            return None;
        };
        current = fields.get(*name)?;
    }
    Some(current)
}

pub(crate) fn text_at(item: &GuardrailValue, path: &[&str]) -> String {
    match nested(item, path) {
        Some(GuardrailValue::Text(text)) => text.clone(),
        Some(other) => panic!("expected text at {path:?}, got {other:?}"),
        None => panic!("no value at {path:?}"),
    }
}

pub(crate) struct FixedStamp(pub(crate) BlockStamp);

impl StampSource for FixedStamp {
    fn stamp(&self) -> BlockStamp {
        self.0.clone()
    }
}

pub(crate) fn stamp() -> FixedStamp {
    FixedStamp(BlockStamp {
        response_id: String::from("resp_fixed"),
        message_id: String::from("msg_fixed"),
        created_at: 1234_i64,
    })
}

pub(crate) fn other_stamp() -> FixedStamp {
    FixedStamp(BlockStamp {
        response_id: String::from("resp_other"),
        message_id: String::from("msg_other"),
        created_at: 99_i64,
    })
}

pub(crate) struct ScriptedAssembler(pub(crate) ChatResponse);

impl ChunkAssembler for ScriptedAssembler {
    fn assemble(
        &self,
        _chunks: &[Object],
    ) -> impl Future<Output = Result<ChatResponse, RewriteError>> + Send {
        ready(Ok(self.0.clone()))
    }
}

pub(crate) fn chat_response(json: &str) -> ChatResponse {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn cache() -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    )
}
