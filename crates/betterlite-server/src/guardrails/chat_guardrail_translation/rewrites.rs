use betterlite_auth::auth_identity::records::Principal;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;

use self::apply_output_texts::apply_output_texts;
use self::apply_output_tool_calls::apply_output_tool_calls;
use self::process_output::{
    extract_response, output_inputs, output_request_data, response_has_text,
};
use self::response::ChatResponse;
use self::structured::tool_call_objects;

pub mod apply_input_texts;
pub mod apply_input_tool_calls;
pub mod apply_output_texts;
pub mod apply_output_tool_calls;
pub mod ended_stream_gather;
pub mod error;
pub mod principal;
pub mod process_input;
pub mod process_output;
pub mod rebuild_ended_stream;
pub mod response;
pub mod string_choice_contents;
pub mod structured;
pub mod tool_call_fragments;
pub mod tool_call_shapes;
pub mod write_text_rewrites;
pub mod write_tool_rewrites;

mod tests;

pub async fn process_output_response<G>(
    response: &mut ChatResponse,
    guardrail: &G,
    principal: &Principal,
) -> Result<(), GuardrailFailure>
where
    G: GuardrailContract,
{
    if !response_has_text(response) {
        tracing::warn!("OpenAI Chat Completions: No text content in response, skipping guardrail");
        return Ok(());
    }
    let extracted = extract_response(response);
    if extracted.texts.is_empty() && extracted.tool_calls.is_empty() {
        return Ok(());
    }
    let mut request_data = output_request_data(response, principal)?;
    let inputs = output_inputs(response, &extracted);
    let guardrailed = guardrail
        .apply_guardrail(
            inputs,
            &mut request_data,
            GuardrailInputType::Response,
            None,
        )
        .await?;
    let guardrailed_texts = guardrailed.texts.unwrap_or_default();
    // A guardrail that changes the number of tool calls is ignored, so the originals stay.
    let tool_calls = match guardrailed.tool_calls {
        Some(returned) if returned.len() == extracted.tool_calls.len() => {
            tool_call_objects(returned)?
        }
        Some(_) | None => extracted.tool_calls.clone(),
    };
    if !guardrailed_texts.is_empty() && !extracted.texts.is_empty() {
        apply_output_texts(response, &guardrailed_texts, &extracted.text_mappings)?;
    }
    if !tool_calls.is_empty() {
        apply_output_tool_calls(response, &tool_calls, &extracted.tool_call_mappings)?;
    }
    Ok(())
}
