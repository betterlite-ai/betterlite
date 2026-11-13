use super::IbmGuardrail;
use super::config::{EventHook, record_applied_guardrail, should_run_guardrail};
use super::data::PromptData;
use super::detection::check_contents;
use super::error::IbmHookError;
use super::response::ModelResponse;

mod tests;

pub async fn async_post_call_success_hook(
    guardrail: &IbmGuardrail,
    data: &mut PromptData,
    response: &ModelResponse,
) -> Result<(), IbmHookError> {
    if !should_run_guardrail(guardrail, EventHook::PostCall) {
        return Ok(());
    }
    let contents: Vec<String> = response
        .choices
        .iter()
        .filter_map(|choice| choice.message.content.as_deref())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .collect();
    if contents.is_empty() {
        tracing::warn!("IBM Guardrail Detector: not running guardrail. No output text in response");
        return Ok(());
    }
    check_contents(guardrail, &contents).await?;
    record_applied_guardrail(data, &guardrail.client.config.guardrail_name);
    Ok(())
}
