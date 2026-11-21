use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::errors::HookFailure;
use super::response::response_has_text;
use super::runtime::{HookRuntime, RequestData, ScanCall, ScanSource};

// LiteLLM's async_post_call_success_hook: OUTPUT scans the model response, skips tool-only replies and keeps its usage on a block.
pub async fn post_call_success_hook<R>(
    runtime: &R,
    data: &mut RequestData,
    response: &mut GuardrailValue,
) -> Result<(), HookFailure>
where
    R: HookRuntime,
{
    if !runtime.should_run(data, GuardrailEventHooks::PostCall) {
        return Ok(());
    }
    let Some(messages) = runtime.request_messages(data) else {
        tracing::warn!("Bedrock AI: not running the guardrail, no messages in data");
        return Ok(());
    };
    if response_has_text(response) == Some(false) {
        tracing::warn!("Bedrock AI: not running the guardrail, no output text in response");
        return Ok(());
    }
    let scan = ScanCall {
        source: ScanSource::Output,
        messages: Some(&messages),
        response: Some(&*response),
        request_data: &mut *data,
        event: GuardrailEventHooks::PostCall,
    };
    let scan_response = match runtime.scan(scan).await {
        Ok(scan_response) => scan_response,
        Err(HookFailure::ModifyResponse(mut failure)) => {
            // The endpoint handler turns the exception into a 200 block reply and needs the usage it replaced.
            if failure.original_response.is_none() {
                failure.original_response = Some(response.clone());
            }
            return Err(HookFailure::ModifyResponse(failure));
        }
        Err(other) => return Err(other),
    };
    runtime.mask_response(response, &scan_response);
    runtime.record_applied(data);
    Ok(())
}
