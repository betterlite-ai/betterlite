use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::batch::BatchInvocation;
use super::masked::response_has_masked_output;
use crate::bedrock_apply::make_bedrock_api_request;
use crate::bedrock_checks::request::ChecksScan;
use crate::bedrock_message_selection::wire::{Content, Message, Role};
use crate::bedrock_records::{Extensions, Response, Source};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_scan_cache::{ScanCacheStore, SessionScanCache as _};

// LiteLLM's _apply_incremental_request_scan: scan new segments, skip when none, fold back on masking.
pub(super) async fn apply_incremental_request_scan<C, T, Store>(
    invocation: &mut BatchInvocation<'_, Store>,
    inputs: &GuardrailApiInputs,
    credentials: &C,
    transport: &T,
) -> Result<Option<GuardrailApiInputs>, GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
    Store: ScanCacheStore,
{
    let texts = inputs.texts.as_deref();
    let new_texts = invocation
        .api
        .base
        .filter_new_texts_for_session(texts, invocation.api.request_data, invocation.cache)
        .await;
    let Some(fresh_texts) = new_texts else {
        return Ok(None);
    };
    if fresh_texts.is_empty() {
        tracing::debug!(
            "Bedrock Guardrail: no new messages to scan for this session, skipping API call"
        );
        return Ok(Some(inputs.clone()));
    }
    let scanned = scan_new_texts(invocation, &fresh_texts, credentials, transport).await?;
    if response_has_masked_output(&scanned) {
        tracing::warn!(
            "Bedrock Guardrail {}: guardrail returned masked/anonymized content; only_scan_new_messages cannot apply masking to skipped context, falling back to a full-context scan",
            invocation
                .api
                .base
                .settings
                .guardrail_name
                .as_deref()
                .unwrap_or("None")
        );
        return Ok(None);
    }
    let base = invocation.api.base;
    base.mark_texts_scanned(
        texts,
        invocation.api.request_data,
        invocation.cache,
        invocation.cache_ttl_seconds,
    )
    .await;
    Ok(Some(inputs.clone()))
}

async fn scan_new_texts<C, T, Store>(
    invocation: &mut BatchInvocation<'_, Store>,
    new_texts: &[String],
    credentials: &C,
    transport: &T,
) -> Result<Response, GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
    Store: ScanCacheStore,
{
    let messages: Vec<Message> = new_texts
        .iter()
        .map(|text| Message {
            role: Some(Role::User),
            content: Some(Content::Text(text.clone())),
            extra: Extensions::new(),
        })
        .collect();
    let scan = ChecksScan {
        source: Source::Input,
        messages: Some(&messages),
        response: None,
        logging_event_type: Some(GuardrailEventHooks::PreCall),
    };
    make_bedrock_api_request(&mut invocation.api, scan, credentials, transport).await
}
