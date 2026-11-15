use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::responses_chat_options::error::ChatOptionsError;
use crate::api::responses_chat_options::records::ChatCompletionRequest;
use crate::api::responses_chat_options::{
    RequestAssembly, RequestContext, chat_completion_request,
};

#[derive(Debug, Error)]
pub enum BatchBodyError {
    #[error("a Responses batch body must be a JSON object")]
    BodyNotObject,
    #[error("Batch record for /v1/responses is missing required `input` field: model={model}")]
    MissingInput { model: String },
    #[error("the Responses batch body cannot be encoded")]
    Encode {
        #[from]
        source: serde_json::Error,
    },
    #[error("the {field} field cannot hold this value")]
    Fields {
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Chat(#[from] ChatOptionsError),
}

pub fn responses_batch_to_chat_body(
    body: &Json,
    custom_llm_provider: Option<&str>,
    assembly: &mut RequestAssembly<'_>,
) -> Result<ChatCompletionRequest, BatchBodyError> {
    let Json::Object(entries) = body else {
        return Err(BatchBodyError::BodyNotObject);
    };
    if matches!(entries.get("input"), None | Some(Json::Null)) {
        return Err(BatchBodyError::MissingInput {
            model: model_text(entries.get("model")),
        });
    }
    let request = typed_request(entries)?;
    let context = RequestContext {
        request: &request,
        provider: custom_llm_provider,
        stream: None,
        extra_headers: None,
        metadata: request.metadata.as_ref().and_then(Option::as_ref),
        service_tier: None,
        api_base: None,
    };
    chat_completion_request(&context, assembly).map_err(BatchBodyError::from)
}

fn model_text(model: Option<&Json>) -> String {
    match model {
        Some(Json::Text(text)) => text.clone(),
        _ => String::new(),
    }
}

fn typed_request(
    entries: &IndexMap<String, Json>,
) -> Result<ResponsesApiRequestParams, BatchBodyError> {
    let model = model_text(entries.get("model"));
    let mut selected = entries.clone();
    selected.insert("model".to_owned(), Json::Text(model));
    let encoded = serde_json::to_vec(&selected)?;
    serde_path_to_error::deserialize(serde_json::Deserializer::from_slice(&encoded))
        .map_err(fields_error)
}

fn fields_error(error: serde_path_to_error::Error<serde_json::Error>) -> BatchBodyError {
    let field = error.path().to_string();
    BatchBodyError::Fields {
        field,
        source: error.into_inner(),
    }
}
