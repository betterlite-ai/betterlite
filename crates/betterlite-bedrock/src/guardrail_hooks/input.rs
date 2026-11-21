use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::RequestData;
use super::errors::HookFailure;
use super::runtime::{HookRuntime, ScanCall, ScanSource};

// Which of LiteLLM's two input hooks runs: async_pre_call_hook or the during-call async_moderation_hook.
#[derive(Debug, Clone, Copy)]
pub enum InputHook {
    PreCall,
    Moderation,
}

// Both input hooks gate on their event and then scan the user messages the same way, writing the masked text back.
pub async fn scan_input<R>(
    runtime: &R,
    data: &mut RequestData,
    call_type: &str,
    hook: InputHook,
) -> Result<(), HookFailure>
where
    R: HookRuntime,
{
    let event = event_for(call_type, hook);
    if !runtime.should_run(data, event) {
        return Ok(());
    }
    scan_user_messages(runtime, data, call_type, event).await
}

// LiteLLM's async_moderation_hook runs the during-call event, or the MCP one for CallTypes::call_mcp_tool; the pre-call hook always runs the pre-call event.
fn event_for(call_type: &str, hook: InputHook) -> GuardrailEventHooks {
    match hook {
        InputHook::PreCall => GuardrailEventHooks::PreCall,
        InputHook::Moderation if call_type == "call_mcp_tool" => GuardrailEventHooks::DuringMcpCall,
        InputHook::Moderation => GuardrailEventHooks::DuringCall,
    }
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
