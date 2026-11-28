use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock_guardrails::bedrock_credentials::CredentialLoadError;
use betterlite_bedrock_guardrails::bedrock_post::{GuardrailTransport, PostError, sign_and_post};
use betterlite_bedrock_guardrails::bedrock_request::PrepareError;
use betterlite_bedrock_guardrails::bedrock_transport::BedrockGuardrail;
use betterlite_bedrock_guardrails::guardrail_error_response::parse_error_response;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_http::proxy::UpstreamResponse;
use secrecy::SecretString;

use self::decide::decide;
use self::record::write_record;
use self::send::{SILENT_LOG, prepare};
use crate::bedrock_checks::messages;
use crate::bedrock_message_selection::wire::{Message, ModelResponse};
use crate::bedrock_records::{Response, Source};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_records::record::{GuardrailStatus, GuardrailTracingDetail};

mod decide;
mod record;
mod send;

#[derive(Debug, Clone, Copy)]
pub struct ChecksEnvironment<'call> {
    pub runtime_endpoint: Option<&'call str>,
    pub region_name: Option<&'call str>,
    pub region: Option<&'call str>,
    pub bearer_token: Option<&'call SecretString>,
}

#[derive(Debug, Clone, Copy)]
pub struct ChecksScan<'call> {
    pub source: Source,
    pub messages: Option<&'call [Message]>,
    pub response: Option<&'call ModelResponse>,
    pub logging_event_type: Option<GuardrailEventHooks>,
}

pub struct ChecksInvocation<'call> {
    pub guardrail: &'call BedrockGuardrail,
    pub base: &'call GuardrailBase,
    pub request_data: &'call mut BTreeMap<String, GuardrailValue>,
    pub log_target: Option<&'call mut GuardrailLogTarget>,
    pub environment: ChecksEnvironment<'call>,
    pub now: fn() -> SystemTime,
}

#[derive(Debug, thiserror::Error)]
pub enum ChecksError {
    #[error("the Bedrock request `api_key` must be a string")]
    ApiKeyType,
    #[error("the Bedrock guardrail credentials cannot be loaded")]
    Credentials(#[from] CredentialLoadError),
    #[error("the Bedrock InvokeGuardrailChecks request cannot be prepared")]
    Prepare(#[from] PrepareError),
    #[error("the Bedrock InvokeGuardrailChecks response cannot be encoded")]
    Encode(#[source] serde_json::Error),
}

impl From<ChecksError> for GuardrailFailure {
    fn from(error: ChecksError) -> Self {
        Self::Http {
            status: 500,
            detail: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RecordTimes {
    started: SystemTime,
    finished: SystemTime,
}

struct RecordOutcome {
    response: GuardrailValue,
    status: GuardrailStatus,
    tracing_detail: Option<GuardrailTracingDetail>,
}

// LiteLLM's _make_invoke_guardrail_checks_request
pub async fn invoke_checks<C, T>(
    invocation: &mut ChecksInvocation<'_>,
    scan: ChecksScan<'_>,
    credentials: &C,
    transport: &T,
) -> Result<Response, GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
{
    let started = (invocation.now)();
    let checks_messages = messages::build(scan.source, scan.messages, scan.response);
    if checks_messages.is_empty() {
        return Ok(Response::default());
    }
    let event_type = scan
        .logging_event_type
        .unwrap_or_else(|| source_event(scan.source));
    let prepared = prepare(invocation, &checks_messages, credentials, started).await?;
    let posted = sign_and_post(
        invocation.guardrail,
        &prepared,
        transport,
        &SILENT_LOG,
        false,
    )
    .await;
    let times = RecordTimes {
        started,
        finished: (invocation.now)(),
    };
    let body = check_body(invocation, event_type, times, posted)?;
    decide(invocation, event_type, times, &body)
}

fn check_body(
    invocation: &mut ChecksInvocation<'_>,
    event_type: GuardrailEventHooks,
    times: RecordTimes,
    posted: Result<UpstreamResponse, PostError>,
) -> Result<Vec<u8>, GuardrailFailure> {
    match posted {
        Ok(response) if response.status == 200 => Ok(response.body),
        Ok(response) => {
            let (status, detail) = parse_error_response(response.status, &response.body);
            Err(fail(invocation, event_type, times, status, detail))
        }
        Err(PostError::Http { status, detail }) => {
            Err(fail(invocation, event_type, times, status, detail))
        }
        Err(
            error @ (PostError::Transport { .. }
            | PostError::Url { .. }
            | PostError::HeaderName { .. }
            | PostError::HeaderValue { .. }),
        ) => {
            let detail = error.to_string();
            Err(fail(invocation, event_type, times, 500, detail))
        }
    }
}

fn fail(
    invocation: &mut ChecksInvocation<'_>,
    event_type: GuardrailEventHooks,
    times: RecordTimes,
    status: u16,
    detail: String,
) -> GuardrailFailure {
    let mut fields = BTreeMap::new();
    fields.insert("error".to_owned(), GuardrailValue::Text(detail.clone()));
    write_record(
        invocation,
        event_type,
        times,
        RecordOutcome {
            response: GuardrailValue::Object(fields),
            status: GuardrailStatus::GuardrailFailedToRespond,
            tracing_detail: None,
        },
    );
    GuardrailFailure::Http { status, detail }
}

const fn source_event(source: Source) -> GuardrailEventHooks {
    match source {
        Source::Input => GuardrailEventHooks::PreCall,
        Source::Output => GuardrailEventHooks::PostCall,
    }
}
