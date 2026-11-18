use std::future::Future;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::headers::AnthropicHeaders;
use crate::response::error::ResponseError;
use crate::response::wire::ResponseEnvelope;
use crate::response::{Logging, RawResponse, ResponseRequest};
use crate::usage::ReasoningTokenCounter;

use super::error::ConfigError;
pub struct TransformRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub optional_params: &'request IndexMap<String, Json>,
    pub litellm_params: &'request IndexMap<String, Json>,
    pub headers: &'request AnthropicHeaders,
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
