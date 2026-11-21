use std::error::Error as _;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock_guardrails::bedrock_post::{
    GuardrailTransport, PostError, TransportLog, sign_and_post,
};
use betterlite_bedrock_guardrails::bedrock_request::{
    PrepareInput, RequestEnvironment, prepare_request,
};
use betterlite_bedrock_guardrails::bedrock_transport::BedrockGuardrail;
use betterlite_bedrock_guardrails::guardrail_error_response::parse_error_response;
use secrecy::SecretString;

use super::error::ApplyError;
use super::record::{LogSite, Logged, write};
use crate::bedrock_checks::request::ChecksEnvironment;
use crate::bedrock_decisions::assessments::should_raise_blocked;
use crate::bedrock_decisions::exceptions::{BlockedGuardrail, blocked_guardrail_failure};
use crate::bedrock_records::{ContentItem, Request, Response, Usage};
use crate::guardrail_base::failure::GuardrailFailure;

// one logical ApplyGuardrail call: the signed request is rebuilt per chunk, the log context spans all of them
pub(super) struct PostCall<'call> {
    pub(super) guardrail: &'call BedrockGuardrail,
    pub(super) environment: ChecksEnvironment<'call>,
    pub(super) body: Request,
    pub(super) credentials: Option<Credentials>,
    pub(super) api_key: Option<SecretString>,
    pub(super) logs: LogSite<'call>,
}

// this path writes the transport record itself from the returned error, like LiteLLM's connection branch
struct SilentLog;

impl TransportLog for SilentLog {
    fn guardrail_failed_to_respond(&self, _detail: &str) {}
}

static SILENT_LOG: SilentLog = SilentLog;

// LiteLLM's _post_apply_guardrail_content: exactly one signed call, a block or an error escaping as ApplyError
pub(super) async fn post<T>(
    call: &mut PostCall<'_>,
    content: &[ContentItem],
    transport: &T,
    completed: &mut Vec<Usage>,
) -> Result<Response, ApplyError>
where
    T: GuardrailTransport,
{
    let mut body = call.body.clone();
    body.content = Some(content.to_vec());
    let prepared = prepare_request(
        call.guardrail,
        PrepareInput {
            environment: RequestEnvironment {
                runtime_endpoint: call.environment.runtime_endpoint,
                bearer_token: call.environment.bearer_token,
            },
            credentials: call.credentials.as_ref(),
            region: call.logs.region,
            api_key: call.api_key.as_ref(),
            extra_headers: &[],
            request_path: None,
            // SigV4 signs each send with the clock at that moment, as LiteLLM's per-send _prepare_request does
            now: (call.logs.now)(),
        },
        &body,
    )?;
    let posted = sign_and_post(call.guardrail, &prepared, transport, &SILENT_LOG, false).await;
    let response_body = match posted {
        Ok(response) if response.status == 200 => response.body,
        Ok(response) => {
            let (status, detail) = parse_error_response(response.status, &response.body);
            return Err(ApplyError::Unreported { status, detail });
        }
        Err(PostError::Http { status, detail }) => {
            return Err(ApplyError::Unreported { status, detail });
        }
        Err(error) => return Err(transport_failure(call, &error)),
    };
    parsed_response(call, &response_body, completed)
}

// LiteLLM's connection branch logs {"error": str(e)} with no usage, since no chunk was billed in this call
fn transport_failure(call: &mut PostCall<'_>, error: &PostError) -> ApplyError {
    let detail = transport_detail(error);
    if let Err(log_error) = write(
        &mut call.logs,
        &Logged::Failure {
            detail: detail.as_str(),
            completed: &[],
        },
    ) {
        tracing::warn!("Bedrock ApplyGuardrail: the failure record was not written: {log_error}");
    }
    ApplyError::Reported(GuardrailFailure::Http {
        status: 500,
        detail,
    })
}

// the transport failure LiteLLM logs is str(e), the innermost cause rather than this crate's wrapper text
fn transport_detail(error: &PostError) -> String {
    error
        .source()
        .map_or_else(|| error.to_string(), ToString::to_string)
}

fn parsed_response(
    call: &mut PostCall<'_>,
    body: &[u8],
    completed: &mut Vec<Usage>,
) -> Result<Response, ApplyError> {
    let parsed: Response = serde_json::from_slice(body).map_err(|source| {
        ApplyError::Reported(GuardrailFailure::Http {
            status: 500,
            detail: source.to_string(),
        })
    })?;
    if should_raise_blocked(&parsed) {
        return Err(blocked(call, body, &parsed, completed));
    }
    if let Some(usage) = parsed.usage.clone() {
        completed.push(usage);
    }
    Ok(parsed)
}

// LiteLLM logs the blocking attempt before it raises, with the usage every earlier chunk already billed
fn blocked(
    call: &mut PostCall<'_>,
    body: &[u8],
    parsed: &Response,
    completed: &[Usage],
) -> ApplyError {
    let logged = Logged::Attempt {
        status_code: 200,
        body,
        response: parsed,
        completed,
    };
    if let Err(error) = write(&mut call.logs, &logged) {
        tracing::warn!("Bedrock ApplyGuardrail: the blocking record was not written: {error}");
    }
    let guardrail = BlockedGuardrail {
        identifier: call.guardrail.guardrail_identifier.as_deref(),
        version: call.guardrail.guardrail_version.as_deref(),
        name: call.logs.base.settings.guardrail_name.as_deref(),
        disable_exception_on_block: call.guardrail.disable_exception_on_block,
    };
    let failure = blocked_guardrail_failure(parsed, call.logs.request_data, &guardrail);
    ApplyError::Reported(failure)
}
