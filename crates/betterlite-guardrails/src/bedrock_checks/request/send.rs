use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_bedrock::aws_settings::bedrock_bearer_token;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock_guardrails::bedrock_credentials::{RegionEnvironment, load_credentials};
use betterlite_bedrock_guardrails::bedrock_post::TransportLog;
use betterlite_bedrock_guardrails::bedrock_request::{
    PrepareInput, RequestEnvironment, prepare_request,
};
use betterlite_bedrock_guardrails::constants::BEDROCK_CHECKS_PATH;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::SecretString;
use serde::Serialize;

use super::{ChecksError, ChecksInvocation};
use crate::bedrock_records::ChecksMessage;

#[derive(Debug, Serialize)]
struct ChecksBody<'body> {
    messages: &'body [ChecksMessage],
    checks: Option<&'body BTreeMap<String, GuardrailValue>>,
}

// the checks caller writes its own failed-to-respond record from the returned error
pub(super) struct SilentLog;

impl TransportLog for SilentLog {
    fn guardrail_failed_to_respond(&self, _detail: &str) {}
}

pub(super) static SILENT_LOG: SilentLog = SilentLog;

pub(super) async fn prepare<C>(
    invocation: &ChecksInvocation<'_>,
    checks_messages: &[ChecksMessage],
    credentials: &C,
    started: SystemTime,
) -> Result<AwsSignedRequest, ChecksError>
where
    C: CredentialSource,
{
    let api_key = request_api_key(invocation.request_data)?;
    let environment = RegionEnvironment {
        region_name: invocation.environment.region_name,
        region: invocation.environment.region,
    };
    let bearer = bedrock_bearer_token(api_key.as_ref(), invocation.environment.bearer_token);
    let (resolved, region) = load_credentials(
        invocation.guardrail,
        bearer.as_ref(),
        environment,
        credentials,
    )
    .await?;
    let body = ChecksBody {
        messages: checks_messages,
        checks: invocation.guardrail.checks.as_ref(),
    };
    prepare_request(
        invocation.guardrail,
        PrepareInput {
            environment: RequestEnvironment {
                runtime_endpoint: invocation.environment.runtime_endpoint,
                bearer_token: invocation.environment.bearer_token,
            },
            credentials: resolved.as_ref(),
            region: &region,
            api_key: api_key.as_ref(),
            extra_headers: &[],
            request_path: Some(BEDROCK_CHECKS_PATH),
            now: started,
        },
        &body,
    )
    .map_err(ChecksError::from)
}

fn request_api_key(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<SecretString>, ChecksError> {
    match request_data.get("api_key") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(SecretString::from(text.clone()))),
        Some(_) => Err(ChecksError::ApiKeyType),
    }
}
