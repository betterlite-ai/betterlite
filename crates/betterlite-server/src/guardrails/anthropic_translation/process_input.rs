#![expect(
    clippy::disallowed_types,
    reason = "Anthropic request bodies, chat rows and guardrail-returned values are provider JSON that betterlite does not model"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

pub mod error;
mod inputs;
mod request;
pub(crate) mod tests;
mod tools;
mod write_back;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailMessage;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    GlobalSkipFlags, GuardrailSkipFlags,
};
use betterlite_prompts::arguments::json::Object as PromptObject;
use serde_json::Value;

use crate::guardrails::anthropic_translation::input_scan::serde_convert;
use crate::guardrails::anthropic_translation::scan_targets::{ScannedText, ScannedToolCall};
use crate::guardrails::tool_call_identity::ToolCallShape;

use error::ProcessInputError;

const MESSAGES: &str = "messages";
const MODEL: &str = "model";
const ROLE: &str = "role";
const SYSTEM: &str = "system";
const TOOLS: &str = "tools";

// What a Python guardrail object carries on itself, which the contract trait does not expose
pub struct InputContext<'ctx> {
    pub skip_flags: GuardrailSkipFlags,
    pub global_flags: GlobalSkipFlags,
    pub guardrail_name: Option<&'ctx str>,
}

struct StructuredView {
    full: Vec<Value>,
    indices: Vec<usize>,
    sent: Vec<GuardrailMessage>,
    hoisted: Option<Value>,
    has_midturn_system_message: bool,
}

#[derive(Default)]
struct Scanned {
    texts: Vec<String>,
    scanned: Vec<ScannedText>,
    images: Vec<String>,
    scanned_tool_calls: Vec<ScannedToolCall>,
    tool_calls: Vec<Value>,
    pre_guardrail_tool_calls: Vec<ToolCallShape>,
}

struct RequestState {
    tools: Vec<PromptObject>,
    structured: StructuredView,
    scanned: Scanned,
    scan_only: bool,
}

struct AnswerContext<'ctx> {
    structured: &'ctx StructuredView,
    scanned: &'ctx Scanned,
    scan_only: bool,
    guardrail_name: Option<&'ctx str>,
    covers_full_request: bool,
}

pub async fn process_input_messages<G>(
    data: &mut Object,
    guardrail: &G,
    context: &InputContext<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
) -> Result<(), ProcessInputError>
where
    G: GuardrailContract,
{
    let Some(messages) = data.get(MESSAGES) else {
        return Ok(());
    };
    if matches!(messages, GuardrailValue::Null) {
        return Ok(());
    }
    let Value::Array(original_messages) = serde_convert(messages, "messages")? else {
        return Err(ProcessInputError::FieldType {
            field: MESSAGES,
            expected: "an array",
        });
    };
    let state = request::prepare_request(data, &original_messages, context).await?;
    let tools = inputs::tools_to_check(state.scan_only, &state.tools)?;
    if state.scanned.texts.is_empty() && state.scanned.tool_calls.is_empty() {
        tracing::debug!("Anthropic Messages: Processed input messages: {original_messages:?}");
        return Ok(());
    }
    let request_inputs =
        inputs::guardrail_inputs(data, &state.scanned, &state.structured.sent, tools)?;
    let answer_context = AnswerContext {
        structured: &state.structured,
        scanned: &state.scanned,
        scan_only: state.scan_only,
        guardrail_name: context.guardrail_name,
        covers_full_request: guardrail.messages_cover_full_request(),
    };
    write_back::run_guardrail(
        data,
        guardrail,
        logging_obj,
        request_inputs,
        &answer_context,
    )
    .await?;
    tracing::debug!("Anthropic Messages: Processed input messages: {original_messages:?}");
    Ok(())
}
