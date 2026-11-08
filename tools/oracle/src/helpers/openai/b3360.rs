use std::collections::BTreeMap;

use betterlite_openai::api::container_contracts::catalog::{CatalogError, container_endpoints};
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
type Prepared = (String, Object);
type Reply<T> = Result<T, ContainerError>;

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

// The cases only reach the BaseContainerConfig defaults, so the required methods stay unimplemented stubs
#[rustfmt::skip]
impl BaseContainerConfig for FixtureConfig {
    fn get_supported_openai_params(&self) -> Vec<String> { Vec::new() }

    fn map_openai_params(&self, _params: &Object, _drop_params: bool) -> Object { Object::new() }

    // ast-grep-ignore: no-long-names
    fn transform_container_create_request(
        &self, _name: &str, _params: &Object, _context: ContainerRequestContext<'_>,
    ) -> Object { Object::new() }

    // ast-grep-ignore: no-long-names
    fn transform_container_create_response(&self, _raw: &[u8]) -> Reply<ContainerObject> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_list_request(
        &self, _base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>,
    ) -> Prepared { (String::new(), Object::new()) }

    // ast-grep-ignore: no-long-names
    fn transform_container_list_response(&self, _raw: &[u8]) -> Reply<ContainerListResponse> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_retrieve_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Prepared { (String::new(), Object::new()) }

    // ast-grep-ignore: no-long-names
    fn transform_container_retrieve_response(&self, _raw: &[u8]) -> Reply<ContainerObject> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_delete_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Prepared { (String::new(), Object::new()) }

    // ast-grep-ignore: no-long-names
    fn transform_container_delete_response(&self, _raw: &[u8]) -> Reply<DeleteContainerResult> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_list_request(
        &self, _id: &str, _base: &str, _context: ContainerRequestContext<'_>, _query: &ContainerListQuery<'_>,
    ) -> Prepared { (String::new(), Object::new()) }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_list_response(&self, _raw: &[u8]) -> Reply<ContainerFileListResponse> {
        Err(ContainerError::ApiBaseRequired)
    }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_content_request(
        &self, _id: &str, _file: &str, _base: &str, _context: ContainerRequestContext<'_>,
    ) -> Prepared { (String::new(), Object::new()) }

    // ast-grep-ignore: no-long-names
    fn transform_container_file_content_response(&self, _raw: &[u8]) -> Reply<Vec<u8>> {
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

impl From<CatalogError> for Box<HelperError> {
    fn from(error: CatalogError) -> Self {
        validation_error(error.to_string())
    }
}

enum Action<'names> {
    Decode,
    Fields(&'names [String]),
}

fn apply_to<T>(
    value: &Value,
    action: &Action<'_>,
    field: fn(&T, &str) -> Option<Value>,
) -> HelperResult
where
    T: DeserializeOwned + serde::Serialize,
{
    match serde_json::from_value::<T>(value.clone()) {
        Ok(record) => match action {
            Action::Decode => Ok(serde_json::to_value(record)?),
            Action::Fields(names) => Ok(Value::Array(
                names
                    .iter()
                    .map(|name| field(&record, name).unwrap_or(Value::Null))
                    .collect(),
            )),
        },
        Err(error) => Err(validation_error(error.to_string())),
    }
}

fn apply(entry: &RecordEntry, action: &Action<'_>) -> HelperResult {
    match entry.kind {
        RecordName::ContainerObject => apply_to(&entry.value, action, ContainerObject::field),
        RecordName::DeleteResult => apply_to(&entry.value, action, DeleteContainerResult::field),
        RecordName::ListResponse => apply_to(&entry.value, action, ContainerListResponse::field),
        RecordName::FileObject => apply_to(&entry.value, action, ContainerFileObject::field),
        RecordName::FileListResponse => {
            apply_to(&entry.value, action, ContainerFileListResponse::field)
        }
        RecordName::DeleteFileResponse => {
            apply_to(&entry.value, action, DeleteContainerFileResponse::field)
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
                .map(|record| apply(record, &Action::Decode))
                .collect::<Result<Vec<Value>, _>>()
                .map(Value::Array),
            Input::RecordField { record, names } => apply(record, &Action::Fields(names)),
            Input::EndpointCatalog => {
                serde_json::to_value(container_endpoints()?).map_err(Box::<HelperError>::from)
            }
            Input::ConfigDefaults(defaults) => Ok(config_defaults(defaults)),
        }
    }
}
