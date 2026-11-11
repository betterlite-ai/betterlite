use std::collections::BTreeMap;
use std::future::ready;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_settings::ErrorHeaders;
use betterlite_bedrock::claude_platform_inputs::error::{PlatformInputError, error_class};
use betterlite_bedrock::claude_platform_inputs::signing::{
    PlatformSignRequest, PlatformSignature, sign_platform_request,
};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::target::RegionInputs;
use serde_json::{Value, json};

use super::input::{ErrorClassCall, SignCall, SigningCall};
use super::ops::{config, invalid};
use crate::helpers::{HelperError, HelperResult};

fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).map_err(invalid)?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or_else(|| invalid("now is out of range"))
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(invalid)
}

fn sign_call(call: &SignCall) -> HelperResult {
    let config = config(&call.environment)?;
    let platform = &config.sections.claude_platform_inputs;
    let bearer = config
        .sections
        .bedrock_settings
        .deployments
        .first()
        .and_then(|deployment| deployment.bearer_token.clone());
    let headers: Vec<(String, Option<String>)> = call
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let request = PlatformSignRequest {
        headers: &headers,
        optional_params: &call.optional_params,
        request_data: &call.request_data,
        api_base: &call.api_base,
        api_key: call.api_key.as_ref(),
        environment_api_key: platform.environment.anthropic_aws_api_key.as_ref(),
        configured_bearer: bearer.as_ref(),
        model: call.model.as_deref(),
        region_inputs: RegionInputs::default(),
        now: epoch(call.now)?,
    };
    let credentials = Credentials {
        access_key_id: call.credentials.access_key_id.clone(),
        secret_access_key: call.credentials.secret_access_key.clone(),
        session_token: call.credentials.session_token.clone(),
    };
    let outcome = runtime()?.block_on(sign_platform_request(request, move |_settings, _region| {
        ready(Ok::<Credentials, CredentialError>(credentials))
    }));
    Ok(signature_json(outcome))
}

fn signature_json(outcome: Result<PlatformSignature, PlatformInputError>) -> Value {
    match outcome {
        Ok(PlatformSignature::Supplied { headers }) => json!({
            "signed": false,
            "headers": headers.iter().cloned().collect::<BTreeMap<_, _>>(),
            "body": Value::Null,
        }),
        Ok(PlatformSignature::Signed(signed)) => json!({
            "signed": true,
            "method": signed.method,
            "url": signed.url,
            "headers": signed.headers.iter().cloned().collect::<BTreeMap<_, _>>(),
            "body": STANDARD.encode(&signed.body),
        }),
        Err(error) => json!({"error": true, "status": error.status_code()}),
    }
}

fn error_class_call(call: &ErrorClassCall) -> Value {
    match error_class(
        call.message.clone(),
        call.status_code,
        Some(ErrorHeaders::Http(call.headers.clone())),
    ) {
        Ok(error) => json!({"ok": true, "status_code": error.error.status_code}),
        Err(_) => json!({"ok": false}),
    }
}

pub(super) fn signing(calls: &[SigningCall]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        outcomes.push(match call {
            SigningCall::Sign(sign) => sign_call(sign)?,
            SigningCall::ErrorClass(error) => error_class_call(error),
        });
    }
    Ok(json!({"calls": outcomes}))
}
