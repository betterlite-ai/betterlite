use std::future::Future;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::headers::AnthropicHeaders;
use crate::request::BillingHeader;
use crate::response::error::ResponseError;
use crate::response::wire::ResponseEnvelope;
use crate::response::{Logging, RawResponse, ResponseRequest};
use crate::usage::ReasoningTokenCounter;

use super::error::ConfigError;
// The caller's request plus the provider identity and billing rule the inherited mapper reads from its own config
pub struct TransformRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub messages: &'request [Json],
    pub optional_params: &'request IndexMap<String, Json>,
    pub litellm_params: &'request IndexMap<String, Json>,
    pub headers: &'request AnthropicHeaders,
    pub billing_header: BillingHeader,
}
// The mapper's body and the headers it updated in place, which the call logs and sends unchanged
pub struct TransformOutcome {
    pub body: IndexMap<String, Json>,
    pub headers: AnthropicHeaders,
}
pub struct TransformResponse<'response, 'raw, 'request> {
    pub envelope: &'response mut ResponseEnvelope,
    pub raw: &'response RawResponse<'raw>,
    pub request: &'response ResponseRequest<'request>,
}
pub trait ChatConfig: Sync {
    fn transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<IndexMap<String, Json>, ConfigError>;
    fn async_transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> impl Future<Output = Result<IndexMap<String, Json>, ConfigError>> + Send;
    // The parent mapper adds optional betas to the headers it was handed, so its outcome carries them to the wire
    fn transform_outcome(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<TransformOutcome, ConfigError> {
        self.transform_request(request)
            .map(|body| TransformOutcome {
                body,
                headers: request.headers.clone(),
            })
    }
    fn async_transform_outcome(
        &self,
        request: &TransformRequest<'_>,
    ) -> impl Future<Output = Result<TransformOutcome, ConfigError>> + Send {
        let headers = request.headers.clone();
        async move {
            let body = self.async_transform_request(request).await?;
            Ok(TransformOutcome { body, headers })
        }
    }
    fn filter_request_betas(
        &self,
        provider: &str,
        headers: &mut IndexMap<String, String>,
        body: &mut IndexMap<String, Json>,
    ) -> Result<(), ConfigError>;
    fn transform_response<Counter>(
        &self,
        response: TransformResponse<'_, '_, '_>,
        logging: Logging<'_>,
        counter: &Counter,
    ) -> Result<(), ResponseError<Counter::Failure>>
    where
        Counter: ReasoningTokenCounter;
}
