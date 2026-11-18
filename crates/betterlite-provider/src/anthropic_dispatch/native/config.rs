use betterlite_anthropic::tool_names::REVERSE_MAP_KEY;
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

impl Config<'_> {
    pub(super) fn install(
        &mut self,
        prepared: PreparedRequest,
        litellm_params: &mut IndexMap<String, Json>,
    ) -> Result<(), serde_json::Error> {
        if !prepared.tool_name_reverse.is_empty() {
            litellm_params.insert(
                REVERSE_MAP_KEY.to_owned(),
                serde_json::from_value(serde_json::to_value(&prepared.tool_name_reverse)?)?,
            );
        }
        self.prepared = Some(prepared);
        Ok(())
    }
}

impl ChatConfig for Config<'_> {
    fn transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<IndexMap<String, Json>, ConfigError> {
        if let Some(prepared) = &self.prepared {
            return Ok(prepared.body.clone());
        }
        *self
            .validated
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Validated {
            messages: request.messages.to_vec(),
            optional: request.optional_params.clone(),
            litellm: request.litellm_params.clone(),
            headers: request.headers.clone(),
        });
        Err(Box::new(PrepareNext))
    }

    fn async_transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> impl Future<Output = Result<IndexMap<String, Json>, ConfigError>> + Send {
        ready(self.transform_request(request))
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
