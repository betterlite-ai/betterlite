use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::errors::HookFailure;
use super::runtime::{HookRuntime, RequestData, ScanCall, ScanSource};

// LiteLLM's async_pre_call_hook: scan the user messages before the call and write the guardrail's masked text back.
pub async fn pre_call_hook<R>(
    runtime: &R,
    data: &mut RequestData,
    call_type: &str,
) -> Result<(), HookFailure>
where
    R: HookRuntime,
{
    scan_for_event(runtime, data, call_type, GuardrailEventHooks::PreCall).await
}

// LiteLLM's async_moderation_hook: the LLM call is already in flight, so a block propagates to the endpoint handler.
pub async fn moderation_hook<R>(
    runtime: &R,
    data: &mut RequestData,
    call_type: &str,
) -> Result<(), HookFailure>
where
    R: HookRuntime,
{
    // CallTypes::call_mcp_tool is the one call type that runs the MCP during-call event.
    let event = if call_type == "call_mcp_tool" {
        GuardrailEventHooks::DuringMcpCall
    } else {
        GuardrailEventHooks::DuringCall
    };
    scan_for_event(runtime, data, call_type, event).await
}

// Both input hooks gate on their event and then scan the user messages the same way.
async fn scan_for_event<R: HookRuntime>(
    runtime: &R,
    data: &mut RequestData,
    call_type: &str,
    event: GuardrailEventHooks,
) -> Result<(), HookFailure> {
    if !runtime.should_run(data, event) {
        return Ok(());
    }
    scan_user_messages(runtime, data, call_type, event).await
}

async fn scan_user_messages<R>(
    runtime: &R,
    data: &mut RequestData,
    call_type: &str,
    event: GuardrailEventHooks,
) -> Result<(), HookFailure>
where
    R: HookRuntime,
{
    let Some(messages) = runtime.messages_for_call_type(call_type, data) else {
        tracing::debug!("no messages found for the call type, skipping the guardrail");
        return Ok(());
    };
    let selection = runtime.prepare_for_role(&messages);
    let Some(payload) = selection.payload.filter(|items| !items.is_empty()) else {
        tracing::debug!("no user-role messages available for the guardrail payload");
        return Ok(());
    };
    let scan = ScanCall {
        source: ScanSource::Input,
        messages: Some(&payload),
        response: None,
        request_data: &mut *data,
        event,
    };
    let response = runtime.scan(scan).await?;
    let updated = runtime.update_messages(&payload, &response);
    let original = selection.original.as_deref().unwrap_or(&messages);
    let merged = runtime.merge_messages(original, &updated, selection.target_indices.as_deref());
    data.insert("messages".to_owned(), GuardrailValue::Array(merged));
    runtime.record_applied(data);
    Ok(())
}
