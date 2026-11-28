use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_settings::bedrock_bearer_token;
use crate::aws_settings::sources::Credentials;
use crate::signing::body::Body;
use crate::signing::endpoint::{RequestHeaders, request_headers};
use crate::signing::headers::set_exact;
use ::url::Url;
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
use std::fmt;
use std::time::SystemTime;
use uuid::Uuid;
pub mod chunks;
pub mod collector;
pub mod error;
pub mod event;
pub mod model_id;
pub mod record;
pub mod reply;
pub mod sdk_call;
mod tests;
pub mod url;
use self::url::PassthroughEnvironment;
use collector::{BedrockEventStreamCollector, StreamLogAssembler};
use error::PassthroughFailure;
use reply::NativeReplyTransform;
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BedrockPassthroughPlan")
            .field("has_credentials", &self.identity.credentials.is_some())
            .field("has_bearer_token", &self.identity.bearer_token.is_some())
            .field("environment", &self.environment)
            .finish_non_exhaustive()
    }
}
fn request_body(context: &PassthroughSigning<'_>) -> Result<Vec<u8>, PassthroughFailure> {
    let data: IndexMap<String, Json> = match context.request_data {
        None => IndexMap::new(),
        Some(params) => serde_json::to_string(params)
            .and_then(|text| serde_json::from_str(&text))
            .map_err(PassthroughFailure::Body)?,
    };
    Ok(dumps_object(&data).into_bytes())
}
fn content_type_pairs(headers: &Headers, bearer: bool) -> Vec<(String, Option<String>)> {
    let mut pairs: Vec<(String, Option<String>)> = headers
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect();
    let has_content_type = pairs
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
    if bearer || !has_content_type {
        set_exact(&mut pairs, "Content-Type", "application/json".to_owned());
    }
    pairs
}
fn supplied_authorization(pairs: &[(String, Option<String>)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .filter_map(|(_, value)| {
            value
                .as_ref()
                .map(|text| ("Authorization".to_owned(), text.clone()))
        })
        .collect()
}
pub(crate) fn signing_region(
    environment: &PassthroughEnvironment,
    context: &PassthroughSigning<'_>,
    bearer: bool,
) -> Result<String, PassthroughFailure> {
    if bearer {
        return Ok(String::new());
    }
    let explicit = url::optional_text(context.litellm_params, "aws_region_name")?;
    llm_region_name(LlmRegion {
        explicit: explicit.as_deref(),
        model: context.model,
        model_id: None,
        environment_region_name: environment.region_name.as_deref(),
        environment_region: environment.region.as_deref(),
        session_region: environment.session_region.as_deref(),
    })
    .map_err(PassthroughFailure::from)
}

// B658's sign_request body, shared with the SDK port: the identity and the signing region are already chosen
pub(crate) struct SigningIdentity<'text> {
    pub(crate) credentials: Option<&'text Credentials>,
    pub(crate) bearer_token: Option<&'text SecretString>,
    pub(crate) now: SystemTime,
}

pub(crate) fn sign_headers(
    headers: Headers,
    context: &PassthroughSigning<'_>,
    identity: &SigningIdentity<'_>,
    region: &str,
) -> Result<(Headers, Option<Vec<u8>>), PassthroughFailure> {
    let body = request_body(context)?;
    let bearer = bedrock_bearer_token(None, identity.bearer_token);
    let pairs = content_type_pairs(&headers, bearer.is_some());
    let supplied = supplied_authorization(&pairs);
    let bounded = pairs.iter().any(|(name, _)| {
        name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("transfer-encoding")
    });
    let signed = request_headers(RequestHeaders {
        credentials: identity.credentials,
        region,
        extra_headers: &supplied,
        endpoint: context.api_base,
        data: Body::Bytes(&body),
        headers: &pairs,
        api_key: None,
        configured_bearer: identity.bearer_token,
        supports_bearer_token: true,
        now: identity.now,
    })?;
    let signed_headers: Headers = signed
        .headers
        .into_iter()
        .filter(|(name, _)| bounded || !name.eq_ignore_ascii_case("content-length"))
        .collect();
    Ok((signed_headers, Some(signed.body)))
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
        url::complete_url(&self.environment, request)
    }
    fn sign_request(
        &self,
        headers: Headers,
        context: &PassthroughSigning<'_>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        let identity = SigningIdentity {
            credentials: self.identity.credentials.as_ref(),
            bearer_token: self.identity.bearer_token.as_ref(),
            now: self.identity.now,
        };
        let bearer = bedrock_bearer_token(None, identity.bearer_token);
        let region = signing_region(&self.environment, context, bearer.is_some())?;
        sign_headers(headers, context, &identity, &region)
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
