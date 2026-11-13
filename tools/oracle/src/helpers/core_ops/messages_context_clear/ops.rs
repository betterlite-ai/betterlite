use std::sync::{Mutex, PoisonError};

use betterlite_core::api::messages_context_clear::constants::{
    CLEAR_TOOL_USES_EDIT_TYPE, CLEARED_TOOL_RESULT_CONTENT, COMPACT_DEFAULT_INSTRUCTIONS,
    COMPACT_DEFAULT_TRIGGER_TOKENS, COMPACT_EDIT_TYPE, COMPACT_MIN_TRIGGER_TOKENS,
    COMPACT_NO_TOOL_CALLS_SUFFIX, COMPACT_SUMMARY_MAX_TOKENS, COMPACT_SUMMARY_SYSTEM_PREFIX,
    DEFAULT_INPUT_TOKENS_TRIGGER, DEFAULT_KEEP_TOOL_USES, SUMMARY_MAX_TOKENS_KEY,
    SUMMARY_MODEL_KEY, SUMMARY_TIMEOUT_SECONDS,
};
use betterlite_core::api::messages_context_clear::counter::{
    TokenCountRequest, TokenCounter, TokenizerCounter,
};
use betterlite_core::api::messages_context_clear::editor::{
    ClearToolUsesOutcome, EditorRequest, apply_clear_tool_uses_20250919,
};
use betterlite_core::api::messages_context_clear::error::{
    ClearToolUsesError, ContextManagementError,
};
use betterlite_core::api::messages_context_clear::result::PolyfillResult;
use betterlite_models::decisions::Json;
use serde::Serialize;
use serde_json::json;
use tokio::runtime::Runtime;

use crate::helpers::{HelperError, HelperResult};

use super::failure;
use super::input::{CounterInput, EditorInput, Input, VisibleEditsInput};
use super::real_counter::real_counter;
use super::scripted::ScriptedCounter;

#[derive(Debug, Serialize)]
struct CallRecord {
    model: String,
    messages: Vec<Json>,
    tools: Option<Vec<Json>>,
    count: usize,
}

struct RecordingCounter<'counter> {
    inner: &'counter dyn TokenCounter,
    calls: &'counter Mutex<Vec<CallRecord>>,
}

impl TokenCounter for RecordingCounter<'_> {
    fn count(&self, request: TokenCountRequest<'_>) -> Result<usize, ClearToolUsesError> {
        let result = self.inner.count(request);
        if let Ok(count) = &result {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(CallRecord {
                    model: request.model.to_owned(),
                    messages: request.messages.to_vec(),
                    tools: request.tools.map(<[Json]>::to_vec),
                    count: *count,
                });
        }
        result
    }
}

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Editor(editor) => run_editor(editor),
        Input::VisibleEdits(visible) => run_visible_edits(visible),
        Input::Constants => Ok(constants_value()),
    }
}

fn run_editor(input: &EditorInput) -> HelperResult {
    let calls = Mutex::new(Vec::new());
    let outcome = match &input.counter {
        CounterInput::Scripted { values } => {
            recorded_editor(input, &ScriptedCounter::new(values), &calls)?
        }
        CounterInput::Real => {
            let runtime = runtime()?;
            let counter = TokenizerCounter {
                counter: real_counter()?,
                runtime: runtime.handle().clone(),
            };
            recorded_editor(input, &counter, &calls)?
        }
    };
    let recorded = calls.into_inner().unwrap_or_else(PoisonError::into_inner);
    Ok(json!({
        "messages": outcome.messages,
        "applied": outcome.applied,
        "warnings": outcome.warnings,
        "calls": recorded,
    }))
}

fn recorded_editor(
    input: &EditorInput,
    inner: &dyn TokenCounter,
    calls: &Mutex<Vec<CallRecord>>,
) -> Result<ClearToolUsesOutcome, Box<HelperError>> {
    let counter = RecordingCounter { inner, calls };
    apply_clear_tool_uses_20250919(
        EditorRequest {
            model: &input.model,
            messages: input.messages.clone(),
            tools: input.tools.as_deref(),
            system: input.system.as_ref(),
            edit_spec: &input.edit_spec,
        },
        &counter,
    )
    .map_err(failure)
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(failure)
}

fn run_visible_edits(input: &VisibleEditsInput) -> HelperResult {
    if let Some(error) = &input.error {
        let management = ContextManagementError {
            status_code: error.status_code,
            message: error.message.clone(),
        };
        return Err(Box::new(HelperError {
            class: "AnthropicContextManagementError".to_owned(),
            status: Some(management.status_code),
            error_type: None,
            param: None,
            code: None,
            message: management.to_string(),
        }));
    }
    let result = PolyfillResult {
        messages: Vec::new(),
        system: None,
        applied_edits: input.applied_edits.clone(),
        compaction_block: input.compaction_block.clone(),
        iterations_usage: None,
    };
    Ok(json!({ "visible": result.applied_edits_for_response() }))
}

fn constants_value() -> serde_json::Value {
    json!({
        "clear_tool_uses_edit_type": CLEAR_TOOL_USES_EDIT_TYPE,
        "default_input_tokens_trigger": DEFAULT_INPUT_TOKENS_TRIGGER,
        "default_keep_tool_uses": DEFAULT_KEEP_TOOL_USES,
        "cleared_tool_result_content": CLEARED_TOOL_RESULT_CONTENT,
        "compact_edit_type": COMPACT_EDIT_TYPE,
        "compact_default_trigger_tokens": COMPACT_DEFAULT_TRIGGER_TOKENS,
        "compact_min_trigger_tokens": COMPACT_MIN_TRIGGER_TOKENS,
        "compact_summary_max_tokens": COMPACT_SUMMARY_MAX_TOKENS,
        "summary_max_tokens_key": SUMMARY_MAX_TOKENS_KEY,
        "summary_timeout_seconds": SUMMARY_TIMEOUT_SECONDS,
        "summary_model_key": SUMMARY_MODEL_KEY,
        "compact_summary_system_prefix": COMPACT_SUMMARY_SYSTEM_PREFIX,
        "compact_default_instructions": COMPACT_DEFAULT_INSTRUCTIONS,
        "compact_no_tool_calls_suffix": COMPACT_NO_TOOL_CALLS_SUFFIX,
    })
}
