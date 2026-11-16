use std::fmt;
use std::time::SystemTime;

use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest,
    RelayedResponse,
};
use betterlite_openai::passthrough::transformation::{
    BasePassthroughConfig, PassthroughStreamCollector,
};
use betterlite_streaming::stream_base_assembly::provider_metadata::ChatConfigSelection;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use url::Url;
use uuid::Uuid;

use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_settings::sources::Credentials;
use crate::signing::body::Body;
use crate::signing::endpoint::{RequestHeaders, request_headers};
use crate::signing::headers::set_exact;

pub mod chunks;
pub mod collector;
pub mod error;
pub mod event;
pub mod model_id;
pub mod record;
pub mod reply;
pub mod url;

#[cfg(test)]
mod tests;

use collector::{BedrockEventStreamCollector, StreamLogAssembler};
use error::PassthroughFailure;
use reply::NativeReplyTransform;
use url::PassthroughEnvironment;

#[derive(Debug, Clone, Copy)]
pub struct CollectorClock {
    pub next_id: fn() -> Uuid,
    pub now: fn() -> i64,
    pub instant: fn() -> StreamInstant,
}

pub struct PassthroughIdentity {
    pub credentials: Option<Credentials>,
    pub bearer_token: Option<SecretString>,
    pub now: SystemTime,
}

pub struct BedrockPassthroughPlan<'services> {
    pub environment: PassthroughEnvironment,
    pub identity: PassthroughIdentity,
    pub selector: &'services dyn ChatConfigSelection,
    pub transform: &'services dyn NativeReplyTransform,
    pub assembler: &'services dyn StreamLogAssembler,
    pub clock: CollectorClock,
}

impl fmt::Debug for BedrockPassthroughPlan<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BedrockPassthroughPlan")
            .field("has_credentials", &self.identity.credentials.is_some())
            .field("has_bearer_token", &self.identity.bearer_token.is_some())
            .field("environment", &self.environment)
            .finish_non_exhaustive()
    }
}

fn decisions_json(value: &impl Serialize) -> Result<Json, PassthroughFailure> {
    serde_json::from_str(&serde_json::to_string(value).map_err(PassthroughFailure::Body)?)
        .map_err(PassthroughFailure::Body)
}

fn decisions_object(params: &Object) -> Result<IndexMap<String, Json>, PassthroughFailure> {
    params
        .iter()
        .map(|(name, value)| Ok((name.clone(), decisions_json(value)?)))
        .collect()
}

// LiteLLM's get_bedrock_bearer_token for the base passthrough config: the deployment api_key never signs
// as a bearer, and only AWS_BEARER_TOKEN_BEDROCK reaches the shared signer as a configured bearer.
#[must_use]
pub fn get_bedrock_bearer_token(litellm_params: &Object) -> Option<SecretString> {
    // the base getter ignores litellm_params.api_key: only a configured AWS_BEARER_TOKEN_BEDROCK signs
    let _ignored_api_key = litellm_params.get("api_key");
    None
}

impl BasePassthroughConfig for BedrockPassthroughPlan<'_> {
    type Error = PassthroughFailure;
    type Logged = Json;

    fn is_streaming_request(&self, endpoint: &str, _request_data: &Object) -> bool {
        endpoint.contains("stream")
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        url::complete_url(self, request)
    }

    fn sign_request(
        &self,
        headers: Headers,
        context: &PassthroughSigning<'_>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        let request_data = match context.request_data {
            None => IndexMap::new(),
            Some(params) => decisions_object(params)?,
        };
        let body = dumps_object(&request_data).into_bytes();
        let api_key = get_bedrock_bearer_token(context.litellm_params);
        let bearer = crate::aws_settings::bedrock_bearer_token(
            api_key.as_ref(),
            self.identity.bearer_token.as_ref(),
        );
        let mut pairs: Vec<(String, Option<String>)> = headers
            .iter()
            .map(|(name, value)| (name.clone(), Some(value.clone())))
            .collect();
        // LiteLLM sets Content-Type by exact key for a bearer and only when absent for SigV4.
        if bearer.is_some() {
            set_exact(&mut pairs, "Content-Type", "application/json".to_owned());
        } else if !pairs
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        {
            pairs.insert(
                0,
                (
                    "Content-Type".to_owned(),
                    Some("application/json".to_owned()),
                ),
            );
        }
        let supplied_authorization: Vec<(String, String)> = pairs
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .filter_map(|(_, value)| {
                value
                    .as_ref()
                    .map(|text| ("Authorization".to_owned(), text.clone()))
            })
            .collect();
        // the bearer branch of the shared signer returns before the region matters
        let region = if bearer.is_some() {
            String::new()
        } else {
            let explicit = url::optional_text(context.litellm_params, "aws_region_name")?;
            llm_region_name(LlmRegion {
                explicit: explicit.as_deref(),
                model: context.model,
                model_id: None,
                environment_region_name: self.environment.region_name.as_deref(),
                environment_region: self.environment.region.as_deref(),
                session_region: self.environment.session_region.as_deref(),
            })?
        };
        let bounded = pairs.iter().any(|(name, _)| {
            name.eq_ignore_ascii_case("content-length")
                || name.eq_ignore_ascii_case("transfer-encoding")
        });
        let signed = request_headers(RequestHeaders {
            credentials: self.identity.credentials.as_ref(),
            region: &region,
            extra_headers: &supplied_authorization,
            endpoint: context.api_base,
            data: Body::Bytes(&body),
            headers: &pairs,
            api_key: api_key.as_ref(),
            configured_bearer: self.identity.bearer_token.as_ref(),
            supports_bearer_token: true,
            now: self.identity.now,
        })?;
        let mut signed_headers = Headers::new();
        for (name, value) in signed.headers {
            if bounded || !name.eq_ignore_ascii_case("content-length") {
                signed_headers.insert(name, value);
            }
        }
        Ok((signed_headers, Some(signed.body)))
    }

    fn get_error_class(&self, message: &str, status_code: u16, headers: &Headers) -> ProviderError {
        error::bedrock_provider_error(message, status_code, headers)
    }

    fn logging_non_streaming_response(
        &self,
        target: &PassthroughTarget<'_>,
        response: &RelayedResponse<'_>,
        request_data: &Object,
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Self::Logged> {
        match reply::logged_reply(self, target, response, request_data) {
            Ok(logged) => logged,
            Err(failure) => {
                tracing::warn!("the Bedrock passthrough reply could not be logged: {failure}");
                None
            }
        }
    }

    fn create_stream_collector(
        &self,
        target: &PassthroughTarget<'_>,
    ) -> Box<dyn PassthroughStreamCollector<Logged = Self::Logged> + Send + '_>
    where
        Self: Sized + Sync,
    {
        let choice = collector::decoder_selection(target.model, target.endpoint, self.clock);
        let decoder = collector::decoder_box(choice);
        Box::new(BedrockEventStreamCollector::new(
            decoder,
            self.assembler,
            self.clock,
        ))
    }
}

impl From<ProviderError> for PassthroughFailure {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error::ProviderFailure(error))
    }
}
