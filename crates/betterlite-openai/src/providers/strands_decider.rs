pub mod arn;
pub mod body;
pub mod headers;
pub mod response;
pub mod signing;

use std::sync::Arc;
use std::time::SystemTime;

use betterlite_models::decisions::ir;

use crate::decisions::transformation::provider_config::{
    DecisionsProviderConfig, ParseResponseError, SigningContext,
};
use crate::openai_base_config::Headers;

#[path = "strands_decider/strands_config/tests.rs"]
mod strands_config;
#[path = "strands_decider/strands_errors/tests.rs"]
mod strands_errors;
#[path = "strands_decider/strands_headers/tests.rs"]
mod strands_headers;
#[path = "strands_decider/strands_http/tests.rs"]
mod strands_http;
#[path = "strands_decider/strands_parsing/tests.rs"]
mod strands_parsing;
#[path = "strands_decider/strands_response/tests.rs"]
mod strands_response;
#[path = "strands_decider/strands_session/tests.rs"]
mod strands_session;
#[path = "strands_decider/strands_signing/tests.rs"]
mod strands_signing;
#[path = "strands_decider/strands_url/tests.rs"]
mod strands_url;

pub type ConfigClock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

pub struct StrandsDeciderDecisionsConfig {
    pub credentials: Option<signing::AwsCredentials>,
    pub clock: ConfigClock,
}

impl Default for StrandsDeciderDecisionsConfig {
    fn default() -> Self {
        Self {
            credentials: None,
            clock: Arc::new(SystemTime::now),
        }
    }
}

struct InheritedDecisionsConfig;

impl DecisionsProviderConfig for InheritedDecisionsConfig {}

impl DecisionsProviderConfig for StrandsDeciderDecisionsConfig {
    const API_KEY_ENV: &'static [&'static str] = &["STRANDS_DECIDER_API_KEY"];
    const API_BASE_ENV: &'static [&'static str] = &["STRANDS_DECIDER_API_BASE"];
    const API_KEY_REQUIRED: bool = false;

    fn get_complete_url(&self, api_base: &str, model: &str) -> String {
        arn::agentcore_runtime(api_base).map_or_else(
            || InheritedDecisionsConfig.get_complete_url(api_base, model),
            |runtime| runtime.invocations_url(),
        )
    }

    fn signs_with_aws(&self, api_base: &str) -> bool {
        arn::agentcore_runtime(api_base).is_some()
    }

    fn sign_request(
        &self,
        headers: Headers,
        context: SigningContext<'_>,
    ) -> (Headers, Option<Vec<u8>>) {
        let Some(runtime) = arn::agentcore_runtime(context.api_base) else {
            return (headers, None);
        };
        let session = headers::runtime_headers(&headers, &runtime, context.litellm_params);
        if context.api_key.is_some() {
            return (session.headers, None);
        }
        if let Some((signed, payload)) = self.signed_request(&context, &runtime, &session.headers) {
            return (signed, Some(payload));
        }
        (session.headers, None)
    }

    fn transform_decisions_response(
        &self,
        model: &str,
        custom_llm_provider: &str,
        raw_response: &[u8],
        request: &ir::DecisionRequest,
    ) -> Result<ir::DecisionResponse, ParseResponseError> {
        response::runtime_error(raw_response).map_or_else(
            || {
                InheritedDecisionsConfig.transform_decisions_response(
                    model,
                    custom_llm_provider,
                    raw_response,
                    request,
                )
            },
            |error| {
                Err(ParseResponseError::Invalid(Box::new(
                    response::runtime_failure(&error),
                )))
            },
        )
    }
}

impl StrandsDeciderDecisionsConfig {
    fn signed_request(
        &self,
        context: &SigningContext<'_>,
        runtime: &arn::AgentCoreRuntime,
        session: &Headers,
    ) -> Option<(Headers, Vec<u8>)> {
        // LiteLLM validates the call's litellm_params into AwsAuthParams first, so deployment keys beat injected ones
        let resolved = signing::credentials_from_params(context.litellm_params)
            .or_else(|| self.credentials.clone());
        let Some(credentials) = resolved else {
            tracing::error!("no AWS credentials are configured for the AgentCore runtime");
            return None;
        };
        let payload = match body::request_body(context.body) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!(error = %error, "the decision request body cannot be written");
                return None;
            }
        };
        let post = signing::JsonPost {
            url: context.url,
            region: &runtime.region,
            body: &payload,
            headers: session,
            credentials: &credentials,
            now: (self.clock)(),
        };
        match signing::sign_json_post(&post) {
            Ok(signed) => Some((signed, payload)),
            Err(error) => {
                tracing::error!(error = %error, "the AgentCore request cannot be signed");
                None
            }
        }
    }
}
