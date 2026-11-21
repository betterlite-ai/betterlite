use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::json;

use super::ContainerOperationRequest;
use super::error::{ContainerOpError, ContainerOpFailure, failure};
use super::handler::ContainerHandlerCall;
use crate::containers::container_request_parameters::ContainerFieldTypeError;
// ast-grep-ignore: no-long-names
use crate::containers::container_request_parameters::encode_container_id_in_response as encode_id;

// the declared default for custom_llm_provider, shared with the mock error path in the module root
pub(crate) const DEFAULT_PROVIDER: &str = "openai";

pub(crate) struct Prepared<'deps> {
    pub(crate) container_id: String,
    pub(crate) provider: String,
    pub(crate) was_encoded: bool,
    pub(crate) config: &'deps dyn BaseContainerConfig,
    pub(crate) litellm_params: Object,
    pub(crate) timeout: Duration,
}

impl Prepared<'_> {
    pub(crate) fn handler_call<'call>(
        &'call self,
        request: &'call ContainerOperationRequest<'_>,
        logging: &'call Mutex<LoggingState>,
    ) -> ContainerHandlerCall<'call> {
        ContainerHandlerCall {
            container_id: &self.container_id,
            config: self.config,
            litellm_params: &self.litellm_params,
            logging,
            extra_headers: request.extra_headers,
            extra_query: request.extra_query,
            timeout: Some(self.timeout),
        }
    }

    // LiteLLM's isinstance-guarded re-encoding: the decoded model_id goes into the metadata, then B3361 encodes
    pub(crate) fn encoded_id(
        &self,
        container_id: &str,
        request: &ContainerOperationRequest<'_>,
        redactor: &Redactor,
    ) -> Result<String, ContainerOpFailure> {
        let mut metadata = request.litellm_metadata.cloned().unwrap_or_default();
        if self.was_encoded
            && let Some(model_id) = self
                .litellm_params
                .get("model_id")
                .and_then(|deployment| deployment.as_str())
        {
            inject_model_info(&mut metadata, model_id).map_err(|error| {
                failure(ContainerOpError::Encode(error), &self.provider, redactor)
            })?;
        }
        let values: IndexMap<String, JsonValue> = metadata
            .iter()
            .map(|(name, value)| (name.clone(), json_value(value)))
            .collect();
        encode_id(container_id, Some(&self.provider), Some(&values), None)
            .map_err(|error| failure(ContainerOpError::Encode(error), &self.provider, redactor))
    }
}

// GenericLiteLLMParams(api_key=api_key, api_base=api_base, api_version=api_version, **kwargs): every extra kwarg, then the named arguments that win
pub(crate) fn call_params(
    request: &ContainerOperationRequest<'_>,
) -> Result<Object, serde_json::Error> {
    let mut params = Object::new();
    if let Some(extras) = request.extras {
        for (name, value) in extras {
            params.insert(name.clone(), serde_json::to_value(value)?);
        }
    }
    if let Some(key) = &request.api_key {
        params.insert("api_key".to_owned(), json!(key.expose_secret()));
    }
    if let Some(base) = request.api_base {
        params.insert("api_base".to_owned(), json!(base));
    }
    if let Some(version) = request.api_version {
        params.insert("api_version".to_owned(), json!(version));
    }
    if let Some(call_id) = request.litellm_call_id {
        params.insert("litellm_call_id".to_owned(), json!(call_id));
    }
    if let Some(metadata) = request.litellm_metadata {
        params.insert(
            "litellm_metadata".to_owned(),
            serde_json::to_value(metadata)?,
        );
    }
    if let Some(mock) = request.mock_response {
        params.insert("mock_response".to_owned(), serde_json::to_value(mock)?);
    }
    Ok(params)
}

// update_from_kwargs(kwargs, model="", optional_params={}, litellm_params={"litellm_call_id": ...}, custom_llm_provider=...)
pub(crate) fn log_call(
    logging: &Mutex<LoggingState>,
    request: &ContainerOperationRequest<'_>,
    provider: &str,
    call_type: &str,
) -> Result<(), LoggingStateError> {
    let mut kwargs = IndexMap::new();
    if let Some(extras) = request.extras {
        for (name, value) in extras {
            kwargs.insert(name.clone(), value.clone());
        }
    }
    if let Some(metadata) = request.litellm_metadata {
        kwargs.insert(
            "litellm_metadata".to_owned(),
            Json::Object(metadata.clone()),
        );
    }
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        request
            .litellm_call_id
            .map_or(Json::Null, |call_id| Json::Text(call_id.to_owned())),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_from_kwargs(
        &mut state,
        &kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(String::new()),
            user: None,
            optional_params: IndexMap::new(),
            additional_params,
        },
    )?;
    call_type.clone_into(&mut state.call_type);
    drop(state);
    Ok(())
}

// litellm_metadata["model_info"]["id"] = litellm_params["model_id"], creating model_info when it is absent
fn inject_model_info(
    fields: &mut IndexMap<String, Json>,
    model_id: &str,
) -> Result<(), ContainerFieldTypeError> {
    let mut model_info = match fields.get("model_info") {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(model_info)) => model_info.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(ContainerFieldTypeError {
                field: "litellm_metadata.model_info",
            });
        }
    };
    model_info.insert("id".to_owned(), Json::Text(model_id.to_owned()));
    fields.insert("model_info".to_owned(), Json::Object(model_info));
    Ok(())
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), json_value(item)))
                .collect(),
        ),
    }
}
