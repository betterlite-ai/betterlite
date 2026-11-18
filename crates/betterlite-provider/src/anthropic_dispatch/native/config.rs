use std::future::ready;
use std::sync::{Mutex, PoisonError};

use betterlite_anthropic::chat_call::config::{ChatConfig, TransformRequest, TransformResponse};
use betterlite_anthropic::chat_call::error::ConfigError;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::request::PreparedRequest;
use betterlite_anthropic::response::{Logging, error::ResponseError, transform_response};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::api::anthropic_beta_filter::matrix_filter::filter_request_betas;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub struct Validated {
    pub messages: Vec<Json>,
    pub optional: IndexMap<String, Json>,
    pub litellm: IndexMap<String, Json>,
    pub headers: AnthropicHeaders,
}

// Stop at B584's validated transform boundary before asynchronous B577 preparation.
#[derive(Debug, thiserror::Error)]
#[error("request validation completed; asynchronous preparation is required")]
struct PrepareNext;

pub struct Config<'matrix> {
    pub validated: Mutex<Option<Validated>>,
    pub prepared: Option<PreparedRequest>,
    pub matrix: &'matrix BetaMatrix,
}

impl ChatConfig for Config<'_> {
    fn transform_request(
        &self,
        input: &TransformRequest<'_>,
    ) -> Result<IndexMap<String, Json>, ConfigError> {
        if let Some(prepared) = &self.prepared {
            return Ok(prepared.body.clone());
        }
        *self
            .validated
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Validated {
            messages: input.messages.to_vec(),
            optional: input.optional_params.clone(),
            litellm: input.litellm_params.clone(),
            headers: input.headers.clone(),
        });
        Err(Box::new(PrepareNext))
    }

    fn async_transform_request(
        &self,
        input: &TransformRequest<'_>,
    ) -> impl Future<Output = Result<IndexMap<String, Json>, ConfigError>> + Send {
        ready(self.transform_request(input))
    }

    fn filter_request_betas(
        &self,
        provider: &str,
        headers: &mut IndexMap<String, String>,
        body: &mut IndexMap<String, Json>,
    ) -> Result<(), ConfigError> {
        if let Some(prepared) = &self.prepared {
            headers.extend(prepared.headers.values.clone());
        }
        let mut typed = serde_json::from_value(serde_json::to_value(&*body)?)?;
        let dropped = filter_request_betas(headers, &mut typed, self.matrix, provider)?;
        for beta in dropped {
            tracing::debug!(token = beta.token, reason = ?beta.reason, "dropped unsupported Anthropic beta");
        }
        *body = serde_json::from_value(serde_json::to_value(typed)?)?;
        Ok(())
    }

    fn transform_response<Counter>(
        &self,
        response: TransformResponse<'_, '_, '_>,
        logging: Logging<'_>,
        counter: &Counter,
    ) -> Result<(), ResponseError<Counter::Failure>>
    where
        Counter: ReasoningTokenCounter,
    {
        transform_response(
            response.envelope,
            response.raw,
            response.request,
            logging,
            counter,
        )
    }
}
