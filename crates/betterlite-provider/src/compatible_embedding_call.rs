use std::time::Duration;

use betterlite_config::sections::compatible_embedding_settings::{Globals, Settings};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::cometapi_wire::embed::{
    complete_url, transform_embedding_request, validate_environment,
};
use betterlite_openai::compatible_embedding::error::EmbeddingError;
use betterlite_openai::compatible_embedding::{
    EmbeddingCall, Mode, finish_embedding, plan_embedding, send_embedding,
};
use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use http::Method;
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use crate::operation::OperationError;
use crate::prepared::body::parse;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest, Upstream};
use crate::transform_retry::TransformRetry;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum CompatibleEmbeddingError {
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    Url(#[source] url::ParseError),
    #[error(transparent)]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Header(#[source] http::Error),
    #[error("the embedding endpoint answered with a stream")]
    Streamed,
}

// main.py's openai_like, llamafile and lm_studio arm: all three share this one resolution and handler
pub struct SharedEmbedding<'source> {
    pub model: &'source str,
    pub input: &'source Json,
    pub api_base: Option<&'source str>,
    pub api_key: Option<&'source SecretString>,
    pub optional_params: &'source IndexMap<String, Json>,
    pub headers: Option<&'source Headers>,
    pub custom_endpoint: bool,
    pub timeout: Duration,
    pub mode: Mode,
    pub globals: &'source Globals,
    pub settings: &'source Settings,
}

impl SharedEmbedding<'_> {
    #[must_use]
    pub fn handler_call(&self) -> EmbeddingCall {
        let (api_base, api_key) =
            self.settings
                .openai_like_identity(self.globals, self.api_base, self.api_key);
        let mut options = self.optional_params.clone();
        if let Some(headers) = self.headers.filter(|headers| !headers.is_empty()) {
            let extra = headers
                .iter()
                .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
                .collect();
            options.insert(String::from("extra_headers"), Json::Object(extra));
        }
        EmbeddingCall {
            model: self.model.to_owned(),
            input: self.input.clone(),
            api_key,
            api_base,
            options,
            headers: None,
            custom_endpoint: self.custom_endpoint,
            timeout: self.timeout,
            mode: self.mode,
        }
    }
}

// B1425's handler: plan the original input, timeout and mode, send through the pooled client, then finish the hooks
pub async fn shared_embedding(
    call: EmbeddingCall,
    client: &ProxiedClient,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<EmbeddingResponse, EmbeddingError> {
    let plan = plan_embedding(call, state, env)?;
    let sent = send_embedding(client, &plan).await;
    finish_embedding(&plan, sent, state, env)
}

pub struct CometapiEmbedding<'source> {
    pub model: &'source str,
    pub input: &'source PromptJson,
    pub optional_params: &'source Object,
    pub api_base: Option<&'source str>,
    pub api_key: Option<&'source SecretString>,
    pub timeout: Duration,
    pub globals: &'source Globals,
    pub settings: &'source Settings,
}

impl CometapiEmbedding<'_> {
    // the branch passes an empty litellm_params map, so nothing from the caller's deployment reaches the request
    pub fn prepared(&self) -> Result<PreparedRequest, CompatibleEmbeddingError> {
        let (api_base, api_key) =
            self.settings
                .cometapi_identity(self.globals, self.api_base, self.api_key);
        let url =
            Url::parse(&complete_url(Some(&api_base))).map_err(CompatibleEmbeddingError::Url)?;
        let headers = validate_environment(
            &Headers::new(),
            api_key.as_ref(),
            &self.settings.cometapi_credentials,
        );
        let body = transform_embedding_request(self.model, self.input, self.optional_params);
        let encoded = serde_json::to_vec(&body).map_err(CompatibleEmbeddingError::Encode)?;
        Ok(PreparedRequest::Embedding(Box::new(PreparedChat {
            http: PreparedHttp {
                method: Method::POST,
                url,
                headers: header_map(&headers).map_err(CompatibleEmbeddingError::Header)?,
                body: encoded,
                deadline: self.timeout,
                body_limit: None,
            },
            retry: TransformRetry::DISABLED,
            client_model: Some(self.model.to_owned()),
        })))
    }

    pub async fn execute(
        &self,
        client: &ProxiedClient,
    ) -> Result<EmbeddingResponse, CompatibleEmbeddingError> {
        let upstream = self.prepared()?.send(client).await?;
        let Upstream::Buffered(response) = upstream else {
            return Err(CompatibleEmbeddingError::Streamed);
        };
        let mut decoded: EmbeddingResponse = parse(response, "embedding")?;
        decoded.model = self.model.to_owned();
        Ok(decoded)
    }
}
