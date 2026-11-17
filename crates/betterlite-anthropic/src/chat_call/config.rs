use std::future::Future;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::headers::AnthropicHeaders;
use crate::response::error::ResponseError;
use crate::response::wire::ResponseEnvelope;
use crate::response::{Logging, RawResponse, ResponseRequest};
use crate::usage::ReasoningTokenCounter;

use super::error::ConfigError;

// What LiteLLM hands the selected provider config: the copied messages, transform_params and the validated headers
pub struct TransformRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub optional_params: &'request IndexMap<String, Json>,
    pub litellm_params: &'request IndexMap<String, Json>,
    pub headers: &'request AnthropicHeaders,
}

// The part of the selected provider chat config this slice calls; B027 supplies the implementation
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
        envelope: &mut ResponseEnvelope,
        raw: &RawResponse<'_>,
        request: &ResponseRequest<'_>,
        logging: Logging<'_>,
        counter: &Counter,
    ) -> Result<(), ResponseError<Counter::Failure>>
    where
        Counter: ReasoningTokenCounter;
}
