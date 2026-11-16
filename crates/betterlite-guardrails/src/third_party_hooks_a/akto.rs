use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use reqwest::StatusCode;
use serde::Serialize;

use super::error::ThirdPartyHookError;
use crate::akto::AktoMirroring;
use crate::akto::error::AktoError;
use crate::akto::payload::{AktoInputs, AktoPayload, build_akto_payload};
use crate::akto::transport::{
    AktoCallFlags, GuardrailVerdict, fire_and_forget_request, handle_guardrail_response,
    send_request,
};
use crate::generic_guardrail_contracts::GuardrailInputType;

const REQUEST_STATUS: u16 = 200;
const BLOCKED_STATUS: u16 = 403;
const BLOCKED_BY: &str = "Akto Proxy";
const BLOCKED_MESSAGE: &str = "Blocked by Akto Guardrails";
const JSON_CONTENT_TYPE: &str = "application/json";

// the marker Akto's ingest reads for a request its guardrails blocked
#[derive(Serialize)]
struct BlockedMarker<'reason> {
    #[serde(rename = "x-blocked-by")]
    blocked_by: &'static str,
    reason: &'reason str,
}

#[derive(Serialize)]
struct ResponseBody {
    body: String,
}

#[derive(Serialize)]
struct ResponseHeaders {
    #[serde(rename = "content-type")]
    content_type: &'static str,
}

pub async fn apply_guardrail(
    guardrail: &AktoMirroring,
    inputs: AktoInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    event_hook: &LitellmParamsMode,
) -> Result<AktoInputs, ThirdPartyHookError> {
    // HOOK_TO_INPUT maps a single pre_call or post_call mode to one input type; a list or tagged
    // mode matches neither, so both input types run, as str(event_hook) does in LiteLLM
    let handled = match event_hook {
        LitellmParamsMode::One(name) if name == "pre_call" => Some(GuardrailInputType::Request),
        LitellmParamsMode::One(name) if name == "post_call" => Some(GuardrailInputType::Response),
        LitellmParamsMode::One(_) | LitellmParamsMode::Many(_) | LitellmParamsMode::Tagged(_) => {
            None
        }
    };
    if let Some(handled) = handled
        && handled != input_type
    {
        return Ok(inputs);
    }
    match input_type {
        GuardrailInputType::Request => check_request(guardrail, inputs, request_data).await,
        GuardrailInputType::Response => send_response(guardrail, inputs, request_data),
    }
}

async fn check_request(
    guardrail: &AktoMirroring,
    inputs: AktoInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<AktoInputs, ThirdPartyHookError> {
    let payload = build_akto_payload(guardrail, &inputs, request_data, REQUEST_STATUS, false)?;
    let verdict = match verdict_of(guardrail, &payload).await {
        Ok(verdict) => verdict,
        Err(error) => return Ok(guardrail.handle_unreachable(inputs, &error)?),
    };
    if verdict.allowed {
        return Ok(inputs);
    }
    block(guardrail, &inputs, request_data, &verdict)?;
    let detail = if verdict.reason.is_empty() {
        BLOCKED_MESSAGE.to_owned()
    } else {
        verdict.reason
    };
    Err(ThirdPartyHookError::Rejection {
        status: StatusCode::FORBIDDEN,
        detail: GuardrailValue::Text(detail),
        openai_code: None,
    })
}

// the blocked request is mirrored for ingest only, and the rejection does not wait for that send
fn block(
    guardrail: &AktoMirroring,
    inputs: &AktoInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
    verdict: &GuardrailVerdict,
) -> Result<(), ThirdPartyHookError> {
    let mut blocked = build_akto_payload(guardrail, inputs, request_data, BLOCKED_STATUS, false)?;
    blocked.response_payload = marker_body(&verdict.reason)?;
    blocked.response_headers = serde_json::to_string(&ResponseHeaders {
        content_type: JSON_CONTENT_TYPE,
    })
    .map_err(|source| AktoError::Encode { source })?;
    let mirror = guardrail.clone();
    guardrail.tasks.spawn(async move {
        fire_and_forget_request(
            &mirror,
            AktoCallFlags {
                guardrails: false,
                ingest_data: true,
            },
            &blocked,
        )
        .await;
    });
    Ok(())
}

fn send_response(
    guardrail: &AktoMirroring,
    inputs: AktoInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<AktoInputs, ThirdPartyHookError> {
    let payload = build_akto_payload(guardrail, &inputs, request_data, REQUEST_STATUS, true)?;
    // the response side never blocks: guardrails and ingest both run in the background
    let mirror = guardrail.clone();
    guardrail.tasks.spawn(async move {
        fire_and_forget_request(
            &mirror,
            AktoCallFlags {
                guardrails: true,
                ingest_data: true,
            },
            &payload,
        )
        .await;
    });
    Ok(inputs)
}

async fn verdict_of(
    guardrail: &AktoMirroring,
    payload: &AktoPayload,
) -> Result<GuardrailVerdict, AktoError> {
    let flags = AktoCallFlags {
        guardrails: true,
        ingest_data: false,
    };
    let answer = send_request(guardrail, flags, payload).await?;
    handle_guardrail_response(&answer)
}

// the payload the blocked mirror carries: responsePayload is the JSON text of {"body": <marker>}
fn marker_body(reason: &str) -> Result<String, AktoError> {
    let marker = serde_json::to_string(&BlockedMarker {
        blocked_by: BLOCKED_BY,
        reason,
    })
    .map_err(|source| AktoError::Encode { source })?;
    serde_json::to_string(&ResponseBody { body: marker })
        .map_err(|source| AktoError::Encode { source })
}
