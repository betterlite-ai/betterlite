use std::collections::BTreeMap;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_core::api::container_file_endpoints::call::{ContainerFileCall, DEFAULT_PROVIDER};
use betterlite_core::api::container_file_list::ContainerFileListRequest;
use betterlite_core::api::container_file_upload::ContainerFileUploadRequest;
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::ContainerFileUpload;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use betterlite_sdk::call_files::files_from_payload;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

mod tests;

const FILE_KEY: &str = "$file";
const CALL_ID: &str = "litellm_call_id";
const MOCK: &str = "mock_response";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SessionWire {
    pub(crate) environment: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct CallWire {
    pub(crate) operation: String,
    pub(crate) settings: SettingsWire,
    // nested, not flattened: arbitrary_precision turns a flattened float into a map
    pub(crate) arguments: Arguments,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    pub(crate) organization: Option<String>,
}

// The keywords one file call takes, named the way Python names them; every other keyword stays in extra
// and becomes the kwargs LiteLLM's own functions read.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Arguments {
    pub(crate) container_id: Option<String>,
    pub(crate) file_id: Option<String>,
    pub(crate) after: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
    file: Option<Json>,
    // the declared default is 600; an explicit zero takes REQUEST_TIMEOUT instead
    pub(crate) timeout: Option<f64>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    // accepted to match the signature, and LiteLLM never forwards it
    extra_body: Option<Json>,
    #[serde(flatten)]
    pub(crate) extra: IndexMap<String, Json>,
}

impl Arguments {
    pub(crate) fn list_request<'args>(
        &'args self,
        extra_query: Option<&'args IndexMap<String, ParamValue>>,
    ) -> ContainerFileListRequest<'args> {
        ContainerFileListRequest {
            container_id: self.container_id.as_deref().unwrap_or_default(),
            after: self.after.as_deref(),
            limit: self.limit,
            order: self.order.as_deref(),
            timeout: self.timeout,
            api_key: self.api_key.clone(),
            api_base: self.api_base.as_deref(),
            api_version: self.api_version.as_deref(),
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_query,
            extra_body: None,
            kwargs: &self.extra,
        }
    }

    pub(crate) fn upload_request<'args>(
        &'args self,
        file: &'args ContainerFileUpload,
        extra_query: Option<&'args IndexMap<String, ParamValue>>,
    ) -> ContainerFileUploadRequest<'args> {
        ContainerFileUploadRequest {
            container_id: self.container_id.as_deref().unwrap_or_default(),
            file,
            timeout: self.timeout,
            api_key: self.api_key.clone(),
            api_base: self.api_base.as_deref(),
            api_version: self.api_version.as_deref(),
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            extra_headers: self.extra_headers.as_ref(),
            extra_query,
            extra_body: None,
            kwargs: &self.extra,
        }
    }

    // Python sends one $file marker, and its bytes are moved out of the call's blobs
    pub(crate) fn upload(
        &self,
        blobs: &mut [Vec<u8>],
    ) -> Result<ContainerFileUpload, ConvertError> {
        let marker = self.file.clone().ok_or(ConvertError::MissingFile)?;
        let mut payload = RequestMap::new();
        payload.insert(FILE_KEY.to_owned(), marker);
        let found = files_from_payload(&payload, blobs).map_err(|_error| ConvertError::File)?;
        let Some((_key, file)) = found.into_iter().next() else {
            return Err(ConvertError::File);
        };
        Ok(ContainerFileUpload {
            file_data: FileData::Tuple3 {
                filename: Some(file.name),
                content: FileBody::Bytes(file.bytes),
                content_type: file.content_type,
            },
        })
    }

    pub(crate) fn endpoint_call(&self) -> Result<ContainerFileCall, ConvertError> {
        Ok(ContainerFileCall {
            container_id: self.container_id.clone(),
            file_id: self.file_id.clone(),
            after: self.after.clone(),
            limit: self.limit,
            order: self.order.clone(),
            file: None,
            timeout: requested_timeout(self.timeout)?,
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            api_version: self.api_version.clone(),
            custom_llm_provider: self
                .custom_llm_provider
                .clone()
                .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned()),
            extra_headers: self.extra_headers.clone(),
            extra_query: query_values(self.extra_query.as_ref())?,
            extra_body: None,
            mock_response: self.extra.get(MOCK).cloned(),
            litellm_call_id: self.call_id()?,
            extra: self.extra.clone(),
        })
    }

    // LiteLLM only reads this keyword out of kwargs, and logging names it as the call id
    pub(crate) fn call_id(&self) -> Result<Option<String>, ConvertError> {
        match self.extra.get(CALL_ID) {
            None | Some(Json::Null) => Ok(None),
            Some(Json::Text(text)) => Ok(Some(text.clone())),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                Err(ConvertError::Field { name: CALL_ID })
            }
        }
    }
}

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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeFilePayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) body: Option<Json>,
    pub(crate) headers: BTreeMap<String, String>,
}

impl NativeFilePayload {
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
    #[error("the {name} argument does not have the declared type")]
    Field { name: &'static str },
    #[error("the upload file argument is not a $file marker")]
    File,
    #[error("the upload file argument is missing")]
    MissingFile,
    #[error("the {operation} operation is not a container file call")]
    Operation { operation: String },
    #[error("the {operation} operation answers raw bytes, not JSON")]
    Binary { operation: String },
    #[error("the {operation} operation answers JSON, not raw bytes")]
    JsonAnswer { operation: String },
}

pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConvertError::Json { source }),
    }
}
