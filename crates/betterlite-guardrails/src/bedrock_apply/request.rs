use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_bedrock::aws_settings::bedrock_bearer_token;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock_guardrails::bedrock_credentials::{RegionEnvironment, load_credentials};
use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_bedrock_guardrails::constants::BEDROCK_DYNAMIC_BODY_DENYLIST;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::SecretString;
use serde_json::value::to_raw_value;

use super::ApiInvocation;
use super::chunking::chunked;
use super::error::{ApplyError, ApplyRequestError};
use super::post::PostCall;
use super::record::{LogSite, Logged, write};
use crate::bedrock_checks::request::ChecksScan;
use crate::bedrock_chunking::{ChunkResult, merge};
use crate::bedrock_message_selection::content;
use crate::bedrock_records::{ContentItem, Request, Response, Source, Usage};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_selection::gate::GuardrailSelection as _;

// LiteLLM's _make_apply_guardrail_request
pub(super) async fn apply_guardrail<C, T>(
    invocation: &mut ApiInvocation<'_>,
    scan: ChecksScan<'_>,
    credentials: &C,
    transport: &T,
) -> Result<Response, GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
{
    let started = (invocation.now)();
    let Some(request) = guard_request(invocation, scan)? else {
        return Ok(Response::default());
    };
    run_apply(invocation, request, started, credentials, transport).await
}

struct ApplyRequest {
    body: Request,
    items: Vec<ContentItem>,
    api_key: Option<SecretString>,
    event_type: GuardrailEventHooks,
}

// LiteLLM converts the scan and merges the dynamic params before it checks whether any content is left
fn guard_request(
    invocation: &ApiInvocation<'_>,
    scan: ChecksScan<'_>,
) -> Result<Option<ApplyRequest>, GuardrailFailure> {
    let mut body = content::convert(
        scan.source,
        scan.messages,
        scan.response,
        invocation.guardrail.contextual_grounding,
    );
    let api_key = dynamic_params(invocation, &mut body)?;
    let event_type = scan
        .logging_event_type
        .unwrap_or_else(|| event_for(scan.source));
    let items = body.content.clone().unwrap_or_default();
    if items.is_empty() {
        // ApplyGuardrail rejects an empty content list with a 400, so a turn with no text is skipped
        return Ok(None);
    }
    Ok(Some(ApplyRequest {
        body,
        items,
        api_key,
        event_type,
    }))
}

async fn run_apply<C, T>(
    invocation: &mut ApiInvocation<'_>,
    request: ApplyRequest,
    started: SystemTime,
    credentials: &C,
    transport: &T,
) -> Result<Response, GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
{
    let guardrail = invocation.guardrail;
    let environment = invocation.environment;
    let bearer = bedrock_bearer_token(request.api_key.as_ref(), environment.bearer_token);
    let region_environment = RegionEnvironment {
        region_name: environment.region_name,
        region: environment.region,
    };
    let (resolved, region) =
        load_credentials(guardrail, bearer.as_ref(), region_environment, credentials)
            .await
            .map_err(ApplyRequestError::from)?;
    let logs = LogSite {
        guardrail_provider: guardrail.guardrail_provider,
        base: invocation.base,
        request_data: &mut *invocation.request_data,
        log_target: invocation.log_target.as_deref_mut(),
        event_type: request.event_type,
        started,
        now: invocation.now,
        catalog: invocation.catalog,
        region: &region,
    };
    let mut call = PostCall {
        guardrail,
        environment,
        body: request.body,
        credentials: resolved,
        api_key: request.api_key,
        logs,
    };
    let mut completed = Vec::new();
    match chunked(&mut call, request.items, transport, &mut completed).await {
        Ok(results) => merged_response(&mut call, &results),
        Err(error) => Err(end_to_end_failure(&mut call, error, &completed)),
    }
}

// LiteLLM merges the guardrail's dynamic body params in, minus the two keys the converted body owns
fn dynamic_params(
    invocation: &ApiInvocation<'_>,
    body: &mut Request,
) -> Result<Option<SecretString>, GuardrailFailure> {
    if invocation.request_data.is_empty() {
        return Ok(None);
    }
    let params = invocation
        .base
        .dynamic_request_body_params(invocation.request_data)
        .map_err(GuardrailFailure::from)?;
    for (key, value) in params {
        if BEDROCK_DYNAMIC_BODY_DENYLIST.contains(&key.as_str()) {
            continue;
        }
        let encoded = to_raw_value(&value).map_err(ApplyRequestError::Encode)?;
        body.extra.insert(key, encoded);
    }
    request_api_key(invocation.request_data)
}

fn request_api_key(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<SecretString>, GuardrailFailure> {
    match request_data.get("api_key") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(SecretString::from(text.clone()))),
        Some(_) => Err(GuardrailFailure::from(ApplyRequestError::ApiKeyType)),
    }
}

const fn event_for(source: Source) -> GuardrailEventHooks {
    match source {
        Source::Input => GuardrailEventHooks::PreCall,
        Source::Output => GuardrailEventHooks::PostCall,
    }
}

fn merged_response(
    call: &mut PostCall<'_>,
    results: &[ChunkResult],
) -> Result<Response, GuardrailFailure> {
    let merged = merge(results).map_err(|error| GuardrailFailure::Http {
        status: 500,
        detail: error.to_string(),
    })?;
    if let Err(error) = write(&mut call.logs, &Logged::Success { merged: &merged }) {
        tracing::warn!("Bedrock ApplyGuardrail: the success record was not written: {error}");
    }
    Ok(merged)
}

// LiteLLM's consolidated catch: one record for the whole logical call, unless the failure logged itself
fn end_to_end_failure(
    call: &mut PostCall<'_>,
    error: ApplyError,
    completed: &[Usage],
) -> GuardrailFailure {
    if let ApplyError::Unreported { detail, .. } = &error
        && let Err(log_error) = write(
            &mut call.logs,
            &Logged::Failure {
                detail: detail.as_str(),
                completed,
            },
        )
    {
        tracing::warn!("Bedrock ApplyGuardrail: the failure record was not written: {log_error}");
    }
    GuardrailFailure::from(error)
}
