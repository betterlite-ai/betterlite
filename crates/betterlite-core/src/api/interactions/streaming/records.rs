use betterlite_interactions::types::response::InteractionsStreamChunk;
use betterlite_models::api_base_lookup::{
    ApiBaseContext, ApiBaseError, ApiBaseParams, get_api_base,
};
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_models::controls::json_value::JsonValue;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::FieldTypeError;
use super::logging::InteractionsStreamLogging;
use crate::api::interactions::contract::records::{InteractionsLogging, Params};

pub struct InteractionsStreamHiddenParams {
    pub model_id: Option<String>,
    pub api_base: Option<String>,
    pub additional_headers: AdditionalHeaders,
}

pub struct InteractionsSuccessCall {
    pub response: InteractionsStreamChunk,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: Option<bool>,
    pub prefer_async_handlers: bool,
}

pub struct InteractionsStreamSetup<'setup, Config, Logging> {
    pub model: Option<&'setup str>,
    pub config: &'setup Config,
    pub logging: &'setup mut Logging,
    pub litellm_metadata: Option<&'setup Params>,
    pub custom_llm_provider: Option<&'setup str>,
    pub api_base: &'setup dyn InteractionsApiBase,
}

pub struct HiddenParamsInput<'input, Logging> {
    pub headers: &'input IndexMap<String, String>,
    pub model: Option<&'input str>,
    pub logging: &'input mut Logging,
    pub metadata: Option<&'input Params>,
    pub api_base: &'input dyn InteractionsApiBase,
}

pub trait InteractionsApiBase {
    fn resolve(&self, model: &str, params: &ApiBaseParams) -> Result<Option<String>, ApiBaseError>;
}

// the shared get_api_base with the model resolution the caller owns
pub struct SharedApiBase<'context> {
    pub context: ApiBaseContext<'context>,
}

impl InteractionsApiBase for SharedApiBase<'_> {
    fn resolve(&self, model: &str, params: &ApiBaseParams) -> Result<Option<String>, ApiBaseError> {
        get_api_base(model, params, &self.context)
    }
}

pub fn hidden_params<Logging: InteractionsStreamLogging>(
    input: HiddenParamsInput<'_, Logging>,
) -> Result<InteractionsStreamHiddenParams, FieldTypeError> {
    let HiddenParamsInput {
        headers,
        model,
        logging,
        metadata,
        api_base: resolver,
    } = input;
    let call_details = logging.logging();
    let api_base = resolved_api_base(call_details, model.unwrap_or_default(), resolver)?;
    Ok(InteractionsStreamHiddenParams {
        model_id: model_id(metadata)?,
        api_base,
        additional_headers: process_response_headers(HeaderSource::Upstream(headers)),
    })
}

fn resolved_api_base(
    logging: &InteractionsLogging,
    model: &str,
    resolver: &dyn InteractionsApiBase,
) -> Result<Option<String>, FieldTypeError> {
    let params = api_base_params(logging)?;
    match resolver.resolve(model, &params) {
        Ok(api_base) => Ok(api_base),
        // LiteLLM's get_api_base turns every failure into None
        Err(error) => {
            tracing::debug!(%error, "the interaction api base could not be resolved");
            Ok(None)
        }
    }
}

fn api_base_params(logging: &InteractionsLogging) -> Result<ApiBaseParams, FieldTypeError> {
    let text = |value: &JsonValue| match value {
        JsonValue::Text(content) => Some(content.clone()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    };
    let flag = |value: &JsonValue| match value {
        JsonValue::Bool(set) => Some(*set),
        JsonValue::Null
        | JsonValue::Number(_)
        | JsonValue::Text(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    };
    let fields = match logging.model_call_details.get("litellm_params") {
        None | Some(JsonValue::Null) => IndexMap::new(),
        Some(JsonValue::Object(fields)) => fields.clone(),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => {
            return Err(FieldTypeError {
                field: "litellm_params",
            });
        }
    };
    Ok(ApiBaseParams {
        api_base: optional_field(&fields, "api_base", "litellm_params.api_base", text)?,
        base_url: optional_field(&fields, "base_url", "litellm_params.base_url", text)?,
        custom_llm_provider: optional_field(
            &fields,
            "custom_llm_provider",
            "litellm_params.custom_llm_provider",
            text,
        )?,
        api_key: optional_field(&fields, "api_key", "litellm_params.api_key", text)?
            .map(SecretString::from),
        vertex_location: optional_field(
            &fields,
            "vertex_location",
            "litellm_params.vertex_location",
            text,
        )?,
        vertex_project: optional_field(
            &fields,
            "vertex_project",
            "litellm_params.vertex_project",
            text,
        )?,
        stream: optional_field(&fields, "stream", "litellm_params.stream", flag)?
            .unwrap_or_default(),
    })
}

fn optional_field<T>(
    fields: &IndexMap<String, JsonValue>,
    name: &str,
    path: &'static str,
    pick: fn(&JsonValue) -> Option<T>,
) -> Result<Option<T>, FieldTypeError> {
    match fields.get(name) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => pick(value).map(Some).ok_or(FieldTypeError { field: path }),
    }
}

fn model_id(metadata: Option<&Params>) -> Result<Option<String>, FieldTypeError> {
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    let Some(model_info) = metadata.get("model_info") else {
        return Ok(None);
    };
    match model_info {
        JsonValue::Null => Ok(None),
        JsonValue::Object(fields) => match fields.get("id") {
            None | Some(JsonValue::Null) => Ok(None),
            Some(JsonValue::Text(id)) => Ok(Some(id.clone())),
            Some(
                JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_),
            ) => Err(FieldTypeError {
                field: "litellm_metadata.model_info.id",
            }),
        },
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_) => {
            Err(FieldTypeError {
                field: "litellm_metadata.model_info",
            })
        }
    }
}
