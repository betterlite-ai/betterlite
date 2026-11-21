use betterlite_azure::passthrough::logging::{NoChatReplyProjection, NoChatStreamAssembly};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_resource_dispatch::passthrough::{self, Call, Outcome};
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::support::{OTHER, RecordingPassthrough, failure, runtime};

// B548's streaming check and complete-URL build, each one read from the leaf's own config
pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let mut streaming = Vec::with_capacity(input.streaming_steps.len());
    for step in &input.streaming_steps {
        let call = Call::StreamingRequest {
            endpoint: &step.endpoint,
            request_data: &step.request_data,
        };
        streaming.push(dispatch_value(&runtime, None, call)?);
    }
    let mut urls = Vec::with_capacity(input.url_steps.len());
    for step in &input.url_steps {
        let request = PassthroughUrlRequest {
            api_base: step.api_base.as_deref(),
            api_key: None,
            target: PassthroughTarget {
                model: &step.model,
                custom_llm_provider: "azure",
                endpoint: &step.endpoint,
            },
            request_query_params: step.request_query_params.as_ref(),
            litellm_params: &step.litellm_params,
        };
        urls.push(dispatch_value(
            &runtime,
            step.env_base.clone(),
            Call::CompleteUrl(request),
        )?);
    }
    Ok(json!({"streaming": streaming, "urls": urls}))
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) streaming_steps: Vec<StreamStep>,
    #[serde(default)]
    pub(super) url_steps: Vec<UrlStep>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamStep {
    pub(super) endpoint: String,
    #[serde(default)]
    pub(super) request_data: Object,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlStep {
    pub(super) model: String,
    pub(super) endpoint: String,
    #[serde(default)]
    pub(super) litellm_params: Object,
    #[serde(default)]
    pub(super) request_query_params: Option<Object>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) env_base: Option<String>,
}

fn config(api_base_env: Option<String>) -> passthrough::Config {
    passthrough::Config {
        api_base_env,
        api_key_env: None,
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: NoChatStreamAssembly,
    }
}

fn dispatch_value(
    runtime: &Runtime,
    env_base: Option<String>,
    call: Call<'_>,
) -> Result<Value, Box<HelperError>> {
    let config = config(env_base);
    let mut logging = RecordingPassthrough::default();
    let result = runtime.block_on(dispatch(AzureResourceOperation::Passthrough(Box::new(
        passthrough::Dispatch {
            config: &config,
            logging: &mut logging,
            call,
        },
    ))));
    match result {
        Ok(AzureResourceResult::Passthrough(Outcome::Streaming(streaming))) => Ok(json!(streaming)),
        Ok(AzureResourceResult::Passthrough(Outcome::Url { url, base })) => {
            Ok(json!({"url": url, "base": base}))
        }
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::Passthrough(error)) => Err(failure(&error)),
        Err(error) => Err(failure(error)),
    }
}
