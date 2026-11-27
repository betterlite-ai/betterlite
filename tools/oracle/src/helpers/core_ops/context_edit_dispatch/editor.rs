use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_core::api::messages_compaction::dispatch::{
    ClearCall, CompactCall, ContextEditor, EditorOutcome,
};
use betterlite_core::api::messages_context_clear::constants::{
    CLEAR_TOOL_USES_EDIT_TYPE, COMPACT_EDIT_TYPE,
};
use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, PolyfillResult, UsageIteration,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

// the class Python raises for a valid request the editor refuses
pub(super) const ANTHROPIC_ERROR: &str = "AnthropicContextManagementError";

// one editor's answer, as the case scripts it: either native return shape or the refusal
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Script {
    Legacy {
        messages: Vec<Json>,
        #[serde(default)]
        applied: Option<AppliedEdit>,
    },
    Polyfill {
        messages: Vec<Json>,
        #[serde(default)]
        system: Option<Json>,
        #[serde(default)]
        applied_edits: Vec<AppliedEdit>,
        #[serde(default)]
        compaction_block: Option<CompactionBlock>,
        #[serde(default)]
        iterations_usage: Option<Vec<UsageIteration>>,
    },
    Error {
        status_code: u16,
        message: String,
    },
}

// what a scripted editor raises; the class decides whether the run reports it or the helper refuses the input
#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct ScriptFailure {
    pub(super) class: String,
    pub(super) status_code: Option<u16>,
    pub(super) message: String,
}

// the injected registry: each call records its arguments and answers the next scripted result
pub(super) struct ScriptedEditor {
    clear: Mutex<VecDeque<Script>>,
    compact: Mutex<VecDeque<Script>>,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedEditor {
    pub(super) fn new(clear: Vec<Script>, compact: Vec<Script>) -> Self {
        Self {
            clear: Mutex::new(VecDeque::from(clear)),
            compact: Mutex::new(VecDeque::from(compact)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, record: Value) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }

    fn next(scripts: &Mutex<VecDeque<Script>>) -> Result<EditorOutcome, ScriptFailure> {
        let script = scripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match script {
            None => Err(ScriptFailure {
                class: String::from("RuntimeError"),
                status_code: None,
                message: String::from("the case ran out of scripts"),
            }),
            Some(Script::Legacy { messages, applied }) => {
                Ok(EditorOutcome::Legacy { messages, applied })
            }
            Some(Script::Polyfill {
                messages,
                system,
                applied_edits,
                compaction_block,
                iterations_usage,
            }) => Ok(EditorOutcome::Polyfilled(PolyfillResult {
                messages,
                system,
                applied_edits,
                compaction_block,
                iterations_usage,
            })),
            Some(Script::Error {
                status_code,
                message,
            }) => Err(ScriptFailure {
                class: String::from(ANTHROPIC_ERROR),
                status_code: Some(status_code),
                message,
            }),
        }
    }
}

impl ContextEditor for ScriptedEditor {
    type Failure = ScriptFailure;

    fn clear_tool_uses(
        &self,
        call: ClearCall,
    ) -> impl Future<Output = Result<EditorOutcome, ScriptFailure>> + Send {
        let record = json!({
            "editor": CLEAR_TOOL_USES_EDIT_TYPE,
            "model": call.model,
            "messages": call.messages,
            "tools": call.tools,
            "system": call.system,
            "edit_spec": call.edit_spec,
        });
        self.record(record);
        let outcome = Self::next(&self.clear);
        ready(outcome)
    }

    fn compact(
        &self,
        call: CompactCall<'_>,
    ) -> impl Future<Output = Result<EditorOutcome, ScriptFailure>> + Send {
        let record = json!({
            "editor": COMPACT_EDIT_TYPE,
            "model": call.model,
            "messages": call.messages,
            "tools": call.tools,
            "system": call.system,
            "edit_spec": call.edit_spec,
            "litellm_metadata": call.litellm_metadata,
            "llm_router": call.llm_router.map(|_router| "router"),
            "user_api_key_auth": call.user_api_key_auth.map(|_principal| "present"),
        });
        self.record(record);
        let outcome = Self::next(&self.compact);
        ready(outcome)
    }
}
