use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::masked::masked_texts;
use super::scan::apply_incremental_request_scan;
use super::selection::{request_messages, select_messages, synthetic_response};
use crate::bedrock_apply::{ApiInvocation, make_bedrock_api_request};
use crate::bedrock_checks::request::ChecksScan;
use crate::bedrock_message_selection::reconcile;
use crate::bedrock_message_selection::wire::Message;
use crate::bedrock_records::{Response, Source};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_scan_cache::ScanCacheStore;

// LiteLLM's apply_guardrail call site: the invocation plus the resolved session cache and its TTL.
pub struct BatchInvocation<'call, Store> {
    pub api: ApiInvocation<'call>,
    pub cache: &'call Store,
    pub cache_ttl_seconds: i64,
    pub tool_results_only: bool,
}

// LiteLLM's apply_guardrail: apply the guardrail to a batch of texts, masking what it rewrote.
pub async fn apply_guardrail<C, T, Store>(
    invocation: &mut BatchInvocation<'_, Store>,
    mut inputs: GuardrailApiInputs,
    input_type: GuardrailInputType,
    credentials: &C,
    transport: &T,
) -> Result<GuardrailApiInputs, GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
    Store: ScanCacheStore,
{
    let count = inputs.texts.as_ref().map_or(0, Vec::len);
    tracing::debug!("Bedrock Guardrail: Applying guardrail to {count} text(s)");
    if matches!(input_type, GuardrailInputType::Request) {
        let incremental =
            apply_incremental_request_scan(invocation, &inputs, credentials, transport)
                .await
                .map_err(wrap_unexpected)?;
        if let Some(scanned) = incremental {
            return Ok(scanned);
        }
    }
    full_scan(invocation, &mut inputs, input_type, credentials, transport)
        .await
        .map_err(wrap_unexpected)?;
    Ok(inputs)
}

async fn full_scan<C, T, Store>(
    invocation: &mut BatchInvocation<'_, Store>,
    inputs: &mut GuardrailApiInputs,
    input_type: GuardrailInputType,
    credentials: &C,
    transport: &T,
) -> Result<(), GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
    Store: ScanCacheStore,
{
    let texts = inputs.texts.clone().unwrap_or_default();
    let selection = select_messages(
        &invocation.api,
        inputs,
        input_type,
        &texts,
        invocation.tool_results_only,
    );
    if selection.skip_scan {
        return Ok(());
    }
    let masked = match selection
        .filtered_messages
        .as_deref()
        .filter(|items| !items.is_empty())
    {
        Some(messages) => {
            let response = scan_request(
                &mut invocation.api,
                messages,
                input_type,
                credentials,
                transport,
            )
            .await?;
            masked_texts(&response)
        }
        None => Vec::new(),
    };
    let merged = reconcile::texts(
        &masked,
        &texts,
        selection.scanned_slice,
        selection.scanned_role_subset,
    );
    tracing::debug!("Bedrock Guardrail: Successfully applied guardrail");
    inputs.texts = Some(merged);
    Ok(())
}

async fn scan_request<C, T>(
    invocation: &mut ApiInvocation<'_>,
    messages: &[Message],
    input_type: GuardrailInputType,
    credentials: &C,
    transport: &T,
) -> Result<Response, GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
{
    let event = event_for(input_type);
    match input_type {
        GuardrailInputType::Request => {
            let scan = ChecksScan {
                source: Source::Input,
                messages: Some(messages),
                response: None,
                logging_event_type: Some(event),
            };
            make_bedrock_api_request(invocation, scan, credentials, transport).await
        }
        GuardrailInputType::Response => {
            let synthetic = synthetic_response(messages)?;
            let context = request_messages(invocation.request_data);
            let scan = ChecksScan {
                source: Source::Output,
                messages: context.as_deref(),
                response: Some(&synthetic),
                logging_event_type: Some(event),
            };
            make_bedrock_api_request(invocation, scan, credentials, transport).await
        }
    }
}

// LiteLLM re-raises HTTP and modify-response exceptions and wraps every other one; betterlite differs.
#[must_use]
pub(super) fn wrap_unexpected(failure: GuardrailFailure) -> GuardrailFailure {
    match failure {
        typed @ (GuardrailFailure::Http { .. }
        | GuardrailFailure::HttpObject { .. }
        | GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)) => typed,
        GuardrailFailure::Other(detail) => {
            tracing::error!("Bedrock Guardrail: Failed to apply guardrail: {detail}");
            GuardrailFailure::Other(format!("Bedrock guardrail failed: {detail}"))
        }
    }
}

#[must_use]
const fn event_for(input_type: GuardrailInputType) -> GuardrailEventHooks {
    match input_type {
        GuardrailInputType::Request => GuardrailEventHooks::PreCall,
        GuardrailInputType::Response => GuardrailEventHooks::PostCall,
    }
}
