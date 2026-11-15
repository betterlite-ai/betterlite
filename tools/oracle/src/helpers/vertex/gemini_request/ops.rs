use std::future::{Future, ready};

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_request::body::{BodyInput, transform};
use betterlite_vertex::gemini_request::error::BodyError;
use betterlite_vertex::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model: String,
    pub(crate) provider: String,
    // the case states what the model catalog answers, so the comparison fixes the capability input
    pub(crate) supports: Supports,
    #[serde(default)]
    pub(crate) modify_params: bool,
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) litellm_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Supports {
    system_message: bool,
    response_schema: bool,
}

impl ModelCapabilityLookup for Input {
    fn supports(
        &self,
        kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        Ok(match kind {
            CapabilityKind::SystemMessages => self.supports.system_message,
            CapabilityKind::ResponseSchema => self.supports.response_schema,
        })
    }
}

struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let provider = match input.provider.as_str() {
        "vertex_ai" => VertexCapabilityProvider::VertexAi,
        "vertex_ai_beta" => VertexCapabilityProvider::VertexAiBeta,
        "gemini" => VertexCapabilityProvider::Gemini,
        other => return Err(format!("unknown provider: {other}")).or_value_error(),
    };
    let mut optional_params = input.optional_params.clone();
    // an explicit cached_content short-circuits LiteLLM's cache stage and reaches the body as an input
    let cached_content = match optional_params.shift_remove("cached_content") {
        None | Some(Json::Null) => None,
        Some(Json::Text(name)) => Some(name),
        Some(_) => {
            let error = BodyError::Field("cached_content", "a string");
            return Ok(json!({"error": error_value(&error)}));
        }
    };
    let body_input = BodyInput {
        provider,
        model: &input.model,
        messages: &input.messages,
        optional_params: &optional_params,
        litellm_params: &input.litellm_params,
        cached_content: cached_content.as_deref(),
        modify_params: input.modify_params,
        schema_template: None,
    };
    let result = runtime.block_on(transform(&body_input, input, &NoFetch));
    Ok(match result {
        Ok(transformed) => {
            json!({"body": transformed.body, "litellm_params": transformed.litellm_params})
        }
        Err(error) => json!({"error": error_value(&error)}),
    })
}

fn error_value(error: &BodyError) -> Value {
    match error {
        BodyError::History(history) => {
            json!({"status": history.status(), "type": history.error_type(), "param": history.param(), "code": history.code()})
        }
        BodyError::Prompt(_) | BodyError::Field(_, _) | BodyError::Encode(_) => {
            json!({"status": 400, "type": null, "param": null, "code": "400", "message": error.to_string()})
        }
    }
}
