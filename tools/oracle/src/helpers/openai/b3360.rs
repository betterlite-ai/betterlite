use std::collections::BTreeMap;

use betterlite_openai::api::container_contracts::catalog::container_endpoints;
use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::api::container_contracts::records::{
    ContainerFileListResponse, ContainerFileObject, ContainerListResponse, ContainerObject,
    DeleteContainerFileResponse, DeleteContainerResult,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

type Object = Map<String, Value>;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordName {
    ContainerObject,
    DeleteResult,
    ListResponse,
    FileObject,
    FileListResponse,
    DeleteFileResponse,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordEntry {
    kind: RecordName,
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorInput {
    message: String,
    status_code: u16,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigDefaultsInput {
    headers: BTreeMap<String, String>,
    api_key: Option<String>,
    api_bases: Vec<Option<String>>,
    error: ErrorInput,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    RecordDecode {
        records: Vec<RecordEntry>,
    },
    RecordField {
        record: RecordEntry,
        names: Vec<String>,
    },
    EndpointCatalog,
    ConfigDefaults(ConfigDefaultsInput),
}

struct FixtureConfig;

impl BaseContainerConfig for FixtureConfig {
    fn get_supported_openai_params(&self) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(&self, _params: &Object, _drop_params: bool) -> Object {
        Object::new()
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_create_request(
        &self,
        _name: &str,
        _params: &Object,
        _context: ContainerRequestContext<'_>,
    ) -> Object {
        Object::new()
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_create_response(
        &self,
        _raw_response: &[u8],
    ) -> Result<ContainerObject, ContainerError> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_list_request(
        &self,
        _api_base: &str,
        _context: ContainerRequestContext<'_>,
        _query: &ContainerListQuery<'_>,
    ) -> (String, Object) {
        (String::new(), Object::new())
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_list_response(
        &self,
        _raw_response: &[u8],
    ) -> Result<ContainerListResponse, ContainerError> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_retrieve_request(
        &self,
        _container_id: &str,
        _api_base: &str,
        _context: ContainerRequestContext<'_>,
    ) -> (String, Object) {
        (String::new(), Object::new())
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_retrieve_response(
        &self,
        _raw_response: &[u8],
    ) -> Result<ContainerObject, ContainerError> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_delete_request(
        &self,
        _container_id: &str,
        _api_base: &str,
        _context: ContainerRequestContext<'_>,
    ) -> (String, Object) {
        (String::new(), Object::new())
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_delete_response(
        &self,
        _raw_response: &[u8],
    ) -> Result<DeleteContainerResult, ContainerError> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_list_request(
        &self,
        _container_id: &str,
        _api_base: &str,
        _context: ContainerRequestContext<'_>,
        _query: &ContainerListQuery<'_>,
    ) -> (String, Object) {
        (String::new(), Object::new())
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_list_response(
        &self,
        _raw_response: &[u8],
    ) -> Result<ContainerFileListResponse, ContainerError> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_content_request(
        &self,
        _container_id: &str,
        _file_id: &str,
        _api_base: &str,
        _context: ContainerRequestContext<'_>,
    ) -> (String, Object) {
        (String::new(), Object::new())
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_content_response(
        &self,
        _raw_response: &[u8],
    ) -> Result<Vec<u8>, ContainerError> {
        Err(ContainerError::ApiBaseRequired)
    }
}

fn validation_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValidationError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl From<serde_json::Error> for Box<HelperError> {
    fn from(error: serde_json::Error) -> Self {
        validation_error(error.to_string())
    }
}

fn round_trip<T>(value: &Value) -> HelperResult
where
    T: DeserializeOwned + serde::Serialize,
{
    match serde_json::from_value::<T>(value.clone()) {
        Ok(record) => Ok(serde_json::to_value(record)?),
        Err(error) => Err(validation_error(error.to_string())),
    }
}

fn decode_record(entry: &RecordEntry) -> HelperResult {
    match entry.kind {
        RecordName::ContainerObject => round_trip::<ContainerObject>(&entry.value),
        RecordName::DeleteResult => round_trip::<DeleteContainerResult>(&entry.value),
        RecordName::ListResponse => round_trip::<ContainerListResponse>(&entry.value),
        RecordName::FileObject => round_trip::<ContainerFileObject>(&entry.value),
        RecordName::FileListResponse => round_trip::<ContainerFileListResponse>(&entry.value),
        RecordName::DeleteFileResponse => round_trip::<DeleteContainerFileResponse>(&entry.value),
    }
}

fn field_list<T>(
    value: &Value,
    names: &[String],
    field: fn(&T, &str) -> Option<Value>,
) -> HelperResult
where
    T: DeserializeOwned,
{
    match serde_json::from_value::<T>(value.clone()) {
        Ok(record) => Ok(Value::Array(
            names
                .iter()
                .map(|name| field(&record, name).unwrap_or(Value::Null))
                .collect(),
        )),
        Err(error) => Err(validation_error(error.to_string())),
    }
}

fn field_values(entry: &RecordEntry, names: &[String]) -> HelperResult {
    match entry.kind {
        RecordName::ContainerObject => field_list(&entry.value, names, ContainerObject::field),
        RecordName::DeleteResult => field_list(&entry.value, names, DeleteContainerResult::field),
        RecordName::ListResponse => field_list(&entry.value, names, ContainerListResponse::field),
        RecordName::FileObject => field_list(&entry.value, names, ContainerFileObject::field),
        RecordName::FileListResponse => {
            field_list(&entry.value, names, ContainerFileListResponse::field)
        }
        RecordName::DeleteFileResponse => {
            field_list(&entry.value, names, DeleteContainerFileResponse::field)
        }
    }
}

fn config_defaults(input: &ConfigDefaultsInput) -> Value {
    let config = FixtureConfig;
    let environment = config.validate_environment(&input.headers, input.api_key.as_deref());
    let urls: Vec<Value> = input
        .api_bases
        .iter()
        .map(|api_base| {
            config
                .get_complete_url(api_base.as_deref(), &Object::new())
                .map_or(Value::Null, Value::String)
        })
        .collect();
    let error = config.get_error_class(
        &input.error.message,
        input.error.status_code,
        &input.error.headers,
    );
    json!({
        "environment": environment,
        "urls": urls,
        "error": {
            "status_code": error.status_code,
            "message": error.message,
            "headers": error.headers,
        },
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::RecordDecode { records } => records
                .iter()
                .map(decode_record)
                .collect::<Result<Vec<Value>, _>>()
                .map(Value::Array),
            Input::RecordField { record, names } => field_values(record, names),
            Input::EndpointCatalog => {
                serde_json::to_value(container_endpoints()).map_err(Box::<HelperError>::from)
            }
            Input::ConfigDefaults(defaults) => Ok(config_defaults(defaults)),
        }
    }
}
