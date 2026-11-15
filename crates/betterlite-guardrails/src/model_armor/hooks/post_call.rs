use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::model_armor::decisions::model_response::{CallResponse, extract_content_from_response};
use crate::model_armor::decisions::{
    build_block_error_detail, get_sanitized_content, should_block_content,
};
use crate::model_armor::transport::ArmorResponse;
use crate::model_armor::transport::tokens::ArmorTokens;
use crate::model_armor::transport::wire::Source;

use super::applied_guardrails;
use super::error_handling::{HookError, call_outcome};
use super::{Data, HookContext, ModelArmorHooks, logging_information, response_masking};

pub(super) async fn scan<T: ArmorTokens>(
    hooks: &ModelArmorHooks<'_, T>,
    data: &mut Data,
    response: &mut CallResponse,
    context: &mut HookContext<'_>,
) -> Result<(), HookError<T::Error>> {
    if !hooks.guardrail.base.should_run_guardrail(
        data,
        GuardrailEventHooks::PostCall,
        context.suppressed,
    )? {
        return Ok(());
    }
    let content = extract_content_from_response(response)?;
    if content.is_empty() {
        tracing::debug!("Model Armor: No text content to process in response, skipping guardrail");
        return Ok(());
    }
    let Some(armor) = scan_response(hooks, &content).await? else {
        return Ok(());
    };
    let mask = hooks.guardrail.base.settings.mask_response_content;
    record(hooks, data, context, &armor, mask)?;
    if mask {
        response_masking::apply(response, &content, get_sanitized_content(&armor.verdict));
    }
    Ok(())
}

async fn scan_response<T: ArmorTokens>(
    hooks: &ModelArmorHooks<'_, T>,
    content: &str,
) -> Result<Option<ArmorResponse>, HookError<T::Error>> {
    let result = hooks
        .transport
        .make_model_armor_request(Some(content), Source::ModelResponse, None)
        .await;
    call_outcome(result, hooks.fail_on_error, "post-call")
}

// LiteLLM writes the standard logging record and the applied header before it decides to block
fn record<T: ArmorTokens>(
    hooks: &ModelArmorHooks<'_, T>,
    data: &mut Data,
    context: &mut HookContext<'_>,
    armor: &ArmorResponse,
    mask: bool,
) -> Result<(), HookError<T::Error>> {
    let blocked = should_block_content(&armor.verdict, mask);
    logging_information::append_post_call(
        hooks,
        data,
        context.model_call_details.as_deref_mut(),
        &armor.body,
        blocked,
    )?;
    applied_guardrails::record(
        data,
        hooks.guardrail.base.settings.guardrail_name.as_deref(),
    );
    if blocked {
        return Err(HookError::Blocked {
            status: 400,
            detail: build_block_error_detail(
                "Response blocked by Model Armor",
                &armor.body,
                hooks.guardrail.sanitize_error_detail,
            ),
        });
    }
    Ok(())
}

mod tests;
