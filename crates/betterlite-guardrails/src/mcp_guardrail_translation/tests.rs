#![cfg(test)]

use super::call_tool_result::CallToolResult;
use super::guardrail::{ApplyGuardrail, GuardrailInputs};
use super::tool_request::McpToolRequest;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::generic_guardrail_contracts::headers::LoggingObjectContext;
use std::future::{self, Future};
use std::sync::Mutex;
use thiserror::Error;
#[derive(Debug, Error)]
#[error("the scripted guardrail failed")]
pub(super) struct ScriptedFailure;

pub(super) enum Reply {
    Mask(&'static str),
    Nothing,
    Texts(Vec<String>),
    Fail,
}

pub(super) struct ScriptedGuardrail {
    pub(super) reply: Reply,
    pub(super) calls: Mutex<Vec<(GuardrailInputs, GuardrailInputType)>>,
}
impl ScriptedGuardrail {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            reply,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<(GuardrailInputs, GuardrailInputType)> {
        self.calls.lock().unwrap().clone()
    }
}
impl ApplyGuardrail for ScriptedGuardrail {
    type Error = ScriptedFailure;

    fn apply_guardrail(
        &self,
        inputs: GuardrailInputs,
        _request_data: Option<&mut McpToolRequest>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut LoggingObjectContext>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, ScriptedFailure>> + Send {
        self.calls
            .lock()
            .unwrap()
            .push((inputs.clone(), input_type));
        let reply = match &self.reply {
            Reply::Mask(secret) => Ok(Some(
                inputs
                    .texts
                    .iter()
                    .map(|text| text.replace(secret, "[MASKED]"))
                    .collect(),
            )),
            Reply::Nothing => Ok(None),
            Reply::Texts(texts) => Ok(Some(texts.clone())),
            Reply::Fail => Err(ScriptedFailure),
        };
        future::ready(reply)
    }
}
pub(super) fn request(json: &str) -> McpToolRequest {
    serde_json::from_str(json).unwrap()
}
pub(super) fn tool_result(json: &str) -> CallToolResult {
    serde_json::from_str(json).unwrap()
}
pub(super) fn to_json<Wire>(wire: &Wire) -> String
where
    Wire: serde::Serialize,
{
    serde_json::to_string(wire).unwrap()
}
pub(super) fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}
