use std::collections::BTreeMap;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_core::api::container_create_list::{
    ContainerCreateCall, ContainerListCall, DEFAULT_PROVIDER,
};
use betterlite_core::api::container_resource_ops::ContainerOperationRequest;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::ContainerObject;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

mod tests;

// The session text: absent means the process environment, the snapshot LiteLLM reads its globals from at import.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SessionWire {
    pub(crate) environment: Option<BTreeMap<String, String>>,
}

// The one text a native method takes: the operation it names, the call's arguments and the globals they fall back to.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct CallWire {
    pub(crate) operation: String,
    pub(crate) settings: SettingsWire,
    // nested, not flattened: arbitrary_precision turns a flattened float into a map
    pub(crate) arguments: Arguments,
}

// litellm.api_base, litellm.api_key, litellm.openai_key and litellm.organization, read at call time.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) organization: Option<String>,
}

// Every keyword the eight functions declare, with the rest of a caller's keyword arguments kept whole.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Arguments {
    pub(crate) name: Option<String>,
    pub(crate) expires_after: Option<Object>,
    pub(crate) file_ids: Option<Vec<String>>,
    pub(crate) after: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
    pub(crate) container_id: Option<String>,
    // the declared default is 600; an explicit zero or none takes LiteLLM's request timeout instead
    pub(crate) timeout: Option<f64>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    // the declared default is "openai"
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    pub(crate) extra_body: Option<BTreeMap<String, String>>,
    pub(crate) litellm_call_id: Option<String>,
    pub(crate) litellm_metadata: Option<IndexMap<String, Json>>,
    pub(crate) mock_response: Option<Json>,
    #[serde(flatten)]
    pub(crate) extra: IndexMap<String, Json>,
}

// Both collection calls carry the same twelve keywords under the same names, so one template spells them
macro_rules! collection_arguments {
    ($call:ident, $arguments:expr, { $($own:ident: $value:expr),* $(,)? }) => {
        $call {
            $($own: $value,)*
            // `timeout or DEFAULT_REQUEST_TIMEOUT` is the operation's own rule, so a zero and a none pass through
            timeout: $arguments.timeout,
            api_key: $arguments.api_key.clone(),
            api_base: $arguments.api_base.clone(),
            api_version: $arguments.api_version.clone(),
            custom_llm_provider: $arguments
                .custom_llm_provider
                .clone()
                .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned()),
            extra_headers: $arguments.extra_headers.clone(),
            extra_query: query_values($arguments.extra_query.as_ref())?,
            extra_body: $arguments.extra_body.clone(),
            mock_response: $arguments.mock_response.clone(),
            litellm_metadata: $arguments.litellm_metadata.as_ref().map(json_values),
            litellm_call_id: $arguments.litellm_call_id.clone(),
            extra: $arguments.extra.clone(),
        }
    };
}

impl Arguments {
    // create_container's keywords, in the shape B3369's operation takes them
    pub(crate) fn create(&self) -> Result<ContainerCreateCall, ConvertError> {
        Ok(collection_arguments!(ContainerCreateCall, self, {
            name: self.name.clone().unwrap_or_default(),
            expires_after: self.expires_after.clone(),
            file_ids: self.file_ids.clone(),
        }))
    }

    // list_containers' keywords: a limit and an order the caller did not set stay absent from the query
    pub(crate) fn list(&self) -> Result<ContainerListCall, ConvertError> {
        Ok(collection_arguments!(ContainerListCall, self, {
            after: self.after.clone(),
            limit: self.limit,
            order: self.order.clone(),
        }))
    }

    // retrieve_container's and delete_container's keywords, with the timeout the operation resolves itself
    pub(crate) fn operation_request<'request>(
        &'request self,
        extra_query: Option<&'request IndexMap<String, ParamValue>>,
        timeout: Option<Duration>,
    ) -> ContainerOperationRequest<'request> {
        ContainerOperationRequest {
            container_id: self.container_id.as_deref().unwrap_or_default(),
            timeout,
            api_key: self.api_key.clone(),
            api_base: self.api_base.as_deref(),
            api_version: self.api_version.as_deref(),
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_query,
            // LiteLLM accepts extra_body on both operations and only passes None on to the response encoder
            extra_body: None,
            litellm_call_id: self.litellm_call_id.as_deref(),
            litellm_metadata: self.litellm_metadata.as_ref(),
            mock_response: self.mock_response.as_ref(),
            extras: Some(&self.extra),
        }
    }
}

// The timeout a caller gave, left for the operation to resolve: LiteLLM's `timeout or DEFAULT_REQUEST_TIMEOUT` reads both none and zero
pub(crate) fn requested_timeout(given: Option<f64>) -> Result<Option<Duration>, ConvertError> {
    let Some(seconds) = given else {
        return Ok(None);
    };
    Duration::try_from_secs_f64(seconds)
        .map(Some)
        .map_err(|source| ConvertError::Timeout { seconds, source })
}

// extra_query crosses as a JSON map; LiteLLM sends a string or a number, and a name says which value failed
pub(crate) fn query_values(
    values: Option<&IndexMap<String, Json>>,
) -> Result<Option<IndexMap<String, ParamValue>>, ConvertError> {
    let Some(given) = values else {
        return Ok(None);
    };
    let mut converted = IndexMap::with_capacity(given.len());
    for (name, value) in given {
        let param = match value {
            Json::Text(text) => ParamValue::Text(text.clone()),
            Json::Number(number) => match number.as_i64() {
                Some(integer) => ParamValue::Integer(integer),
                None => return Err(ConvertError::QueryValue { name: name.clone() }),
            },
            Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => {
                return Err(ConvertError::QueryValue { name: name.clone() });
            }
        };
        converted.insert(name.clone(), param);
    }
    Ok(Some(converted))
}

// The Json values B3369 and B3370 read for litellm_metadata, which they type as the JSON model
fn json_values(metadata: &IndexMap<String, Json>) -> IndexMap<String, JsonValue> {
    let mut values = IndexMap::with_capacity(metadata.len());
    for (name, value) in metadata {
        values.insert(name.clone(), json_value(value));
    }
    values
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(json_values(fields)),
    }
}

// The created container as Python sees it: LiteLLM's record plus the hidden params its create transform set.
#[derive(Debug, Serialize)]
pub(crate) struct CreatedAnswer<'answer> {
    #[serde(flatten)]
    pub(crate) container: &'answer ContainerObject,
    #[serde(rename = "_hidden_params")]
    pub(crate) hidden_params: &'answer Object,
}

// The JSON text a NativeContainerError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeContainerPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) body: Option<Json>,
    pub(crate) headers: BTreeMap<String, String>,
}

impl NativeContainerPayload {
    pub(crate) fn from_class(class: &ErrorClass) -> Self {
        let metadata = class.metadata();
        let mut headers = BTreeMap::new();
        if let ErrorClass::RateLimit(details) = class {
            headers.clone_from(&details.headers);
        }
        Self {
            kind: class_name(class).to_owned(),
            message: class_message(class),
            // a plain Exception carries no status_code; every LiteLLM error class does
            status_code: class_status(class),
            llm_provider: metadata.provider,
            model: metadata.model,
            body: None,
            headers,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the timeout of {seconds} seconds is not a usable duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the {name} query value is not a string or an integer")]
    QueryValue { name: String },
    #[error("the {operation} operation is not a container call")]
    Operation { operation: String },
}

// the one boundary every native method crosses: the text into the shape that method takes
pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConvertError::Json { source }),
    }
}
