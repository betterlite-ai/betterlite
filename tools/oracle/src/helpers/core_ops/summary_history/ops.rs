use std::sync::{Mutex, PoisonError};

use betterlite_core::api::summary_history::boundary::{
    slice_around_compaction_block, strip_compaction_blocks,
};
use betterlite_core::api::summary_history::client_history::client_compaction_history;
use betterlite_core::api::summary_history::count::{TokenComponents, count_effective_tokens};
use betterlite_core::api::summary_history::last_question::select_last_user_question;
use betterlite_core::api::summary_history::metadata::propagate_metadata;
use betterlite_core::api::summary_history::prompt::{build_summary_prompt, extract_summary_text};
use betterlite_core::api::summary_history::summary_messages::build_summary_messages;
use betterlite_core::api::summary_history::system::augment_system_with_summary;
use betterlite_core::api::summary_history::trigger::resolve_trigger_tokens;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::json;
use tokio::runtime::Builder;

use super::counter::{CountSource, RecordingCounter};
use super::input::{
    ClientHistoryInput, CountInput, Input, PromptsInput, SlicingInput, TriggerInput,
};
use super::logs::captured;
use super::translator::CaseTranslator;

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Slicing(slicing) => Ok(slicing_value(slicing)),
        Input::Trigger(trigger) => trigger_value(trigger),
        Input::Prompts(prompts) => prompts_value(prompts),
        Input::Metadata(metadata) => Ok(json!({
            "metadata": propagate_metadata(metadata.parent.as_ref()),
        })),
        Input::Count(count) => count_value(count),
        Input::Extraction(extraction) => Ok(json!({
            "summary": extract_summary_text(extraction.raw.as_deref()).or_value_error()?,
        })),
        Input::ClientHistory(history) => client_history_value(history),
    }
}

fn slicing_value(input: &SlicingInput) -> serde_json::Value {
    let (messages, block) = match slice_around_compaction_block(&input.messages) {
        Some(slice) => (slice.messages, Some(slice.block)),
        None => (input.messages.clone(), None),
    };
    json!({
        "messages": messages,
        "block": block,
        "stripped": strip_compaction_blocks(&input.messages),
        "last_question": select_last_user_question(&input.messages),
        "input": input.messages,
    })
}

fn trigger_value(input: &TriggerInput) -> HelperResult {
    match resolve_trigger_tokens(input.edit_spec.get("trigger")) {
        Ok(resolved) => Ok(json!({ "tokens": resolved.tokens, "warnings": resolved.warnings })),
        Err(error) => Err(Box::new(HelperError {
            class: "AnthropicContextManagementError".to_owned(),
            status: Some(error.status_code),
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}

fn prompts_value(input: &PromptsInput) -> HelperResult {
    let prompt = build_summary_prompt(input.edit_spec.get("instructions"), input.tools.as_deref());
    let augmented = input
        .summary
        .as_deref()
        .map(|summary| augment_system_with_summary(input.system.as_ref(), summary));
    let translator = CaseTranslator {
        translation: input.translation,
    };
    let runtime = Builder::new_current_thread().build()?;
    let (messages, logs) = captured(|| {
        runtime.block_on(build_summary_messages(
            &input.messages,
            &prompt,
            input.system.as_ref(),
            &translator,
        ))
    });
    Ok(json!({
        "prompt": prompt,
        "augmented_system": augmented,
        "summary_messages": messages,
        "logs": logs,
    }))
}

fn count_value(input: &CountInput) -> HelperResult {
    let counter = RecordingCounter {
        source: CountSource::from_case(&input.counter)?,
        calls: Mutex::new(Vec::new()),
    };
    let translator = CaseTranslator {
        translation: input.translation,
    };
    let components = TokenComponents {
        model: &input.model,
        messages: &input.messages,
        compaction_block: input.compaction_block.as_ref(),
        tools: input.tools.as_deref(),
        system: input.system.as_ref(),
    };
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let (counted, logs) =
        captured(|| runtime.block_on(count_effective_tokens(components, &translator, &counter)));
    let total = counted.or_value_error()?;
    let calls = counter
        .calls
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(json!({ "total": total, "calls": calls, "logs": logs }))
}

fn client_history_value(input: &ClientHistoryInput) -> HelperResult {
    let (applied, logs) =
        captured(|| client_compaction_history(&input.messages, input.system.as_ref()));
    let result = applied.or_value_error()?.map(|polyfill| {
        json!({
            "messages": polyfill.messages,
            "system": polyfill.system,
            "applied_edits": polyfill.applied_edits,
            "compaction_block": polyfill.compaction_block,
            "iterations_usage": polyfill.iterations_usage,
        })
    });
    Ok(json!({ "result": result, "logs": logs }))
}
