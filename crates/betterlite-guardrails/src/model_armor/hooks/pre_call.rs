use betterlite_prompts::message_fields::last_user::get_last_user_message;

use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::model_armor::decisions::{
    build_block_error_detail, build_logging_response, get_sanitized_content, should_block_content,
};
use crate::model_armor::request_files::FileScanSettings;
use crate::model_armor::transport::tokens::ArmorTokens;
use crate::model_armor::transport::wire::Source;

use super::applied_guardrails;
use super::error_handling::{HookError, call_outcome};
use super::events::HookPhase;
use super::{Data, HookContext, ModelArmorHooks, metadata, request_masking};

pub(super) async fn scan<T>(
    hooks: &ModelArmorHooks<'_, T>,
    data: &mut Data,
    phase: HookPhase,
    call_type: Option<&str>,
    context: &HookContext<'_>,
) -> Result<(), HookError<T::Error>>
where
    T: ArmorTokens,
{
    let guardrail = hooks.guardrail;
    let event_type = phase.event_type(call_type);
    if !guardrail
        .base
        .should_run_guardrail(data, event_type, context.suppressed)?
    {
        return Ok(());
    }
    let Some(messages) = request_masking::read_messages(data)? else {
        tracing::warn!("Model Armor: not running guardrail. No messages in data");
        return Ok(());
    };
    let settings = FileScanSettings {
        guardrail_name: guardrail.base.settings.guardrail_name.clone(),
        fail_on_error: hooks.fail_on_error,
        skip_unscannable_attachments: hooks.skip_unscannable_attachments,
    };
    metadata::scan_files(data, &messages, hooks.transport, &settings).await?;
    let Some(text) = get_last_user_message(&messages) else {
        return Ok(());
    };
    let sanitized = scan_text(hooks, data, &text, phase.hook_name()).await?;
    if guardrail.base.settings.mask_request_content {
        request_masking::apply(data, &messages, &text, sanitized);
    }
    Ok(())
}

// LiteLLM's try block for the prompt scan: record the verdict, mark the guardrail applied, then block
async fn scan_text<T>(
    hooks: &ModelArmorHooks<'_, T>,
    data: &mut Data,
    text: &str,
    hook: &'static str,
) -> Result<Option<String>, HookError<T::Error>>
where
    T: ArmorTokens,
{
    let guardrail = hooks.guardrail;
    let result = hooks
        .transport
        .make_model_armor_request(Some(text), Source::UserPrompt, None)
        .await;
    let Some(armor) = call_outcome(result, hooks.fail_on_error, hook)? else {
        return Ok(None);
    };
    let mask = guardrail.base.settings.mask_request_content;
    let blocked = should_block_content(&armor.verdict, mask);
    metadata::record_text_scan(
        data,
        build_logging_response(&armor.body, guardrail.sanitize_error_detail),
        blocked,
    );
    applied_guardrails::record(data, guardrail.base.settings.guardrail_name.as_deref());
    if blocked {
        return Err(HookError::Blocked {
            status: 400,
            detail: build_block_error_detail(
                "Content blocked by Model Armor",
                &armor.body,
                guardrail.sanitize_error_detail,
            ),
        });
    }
    Ok(get_sanitized_content(&armor.verdict))
}

mod tests;
