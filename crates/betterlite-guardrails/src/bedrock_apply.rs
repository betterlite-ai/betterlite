use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;
use betterlite_bedrock_guardrails::bedrock_transport::BedrockGuardrail;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::catalog::ModelCatalog;

use self::request::apply_guardrail;
use crate::bedrock_checks::request::{
    ChecksEnvironment, ChecksInvocation, ChecksScan, invoke_checks,
};
use crate::bedrock_records::Response;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailLogTarget;

mod chunking;
mod error;
mod post;
mod record;
mod request;
mod retry;
mod tests;

// one Bedrock guardrail call's inputs, shared by the InvokeGuardrailChecks and ApplyGuardrail paths
pub struct ApiInvocation<'call> {
    pub guardrail: &'call BedrockGuardrail,
    pub base: &'call GuardrailBase,
    pub request_data: &'call mut BTreeMap<String, GuardrailValue>,
    pub log_target: Option<&'call mut GuardrailLogTarget>,
    pub environment: ChecksEnvironment<'call>,
    pub catalog: &'call ModelCatalog,
    pub now: fn() -> SystemTime,
}

// LiteLLM's make_bedrock_api_request: a configured `checks` map routes to InvokeGuardrailChecks, otherwise ApplyGuardrail
pub async fn make_bedrock_api_request<C, T>(
    invocation: &mut ApiInvocation<'_>,
    scan: ChecksScan<'_>,
    credentials: &C,
    transport: &T,
) -> Result<Response, GuardrailFailure>
where
    C: CredentialSource,
    T: GuardrailTransport,
{
    if invocation.guardrail.checks.is_some() {
        let mut checks = ChecksInvocation {
            guardrail: invocation.guardrail,
            base: invocation.base,
            request_data: &mut *invocation.request_data,
            log_target: invocation.log_target.as_deref_mut(),
            environment: invocation.environment,
            now: invocation.now,
        };
        return invoke_checks(&mut checks, scan, credentials, transport).await;
    }
    apply_guardrail(invocation, scan, credentials, transport).await
}
