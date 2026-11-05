use betterlite_core::api::vector_file_contracts::{FileParamsKind, filter_file_params};
use betterlite_openai::api::vector_file_contracts::wire::{
    VectorFileRecordKind, WireDecodeLimits, decode_vector_file_contracts,
};
use betterlite_provider::api::vector_file_contracts::config::{
    FileOperation, Headers, SignRequest, VectorStoreFilesConfig as _,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use minimal_config::MinimalConfig;

mod minimal_config;
mod tests;

type Object = Map<String, Value>;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecordName {
    FileObject,
    CreateRequest,
    UpdateRequest,
    ListQuery,
    ListResponse,
    DeleteResponse,
    ContentResponse,
    ContentTextPart,
    AuthCredentials,
    ChunkingStrategy,
    StaticChunking,
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
    headers: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DefaultsInput {
    optional_params: Object,
    non_default_params: Object,
    headers: Headers,
    request_data: Object,
    api_base: String,
    api_key: Option<String>,
    api_bases: Vec<Option<String>>,
    error: ErrorInput,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    AttachmentParameterFilters {
        create: Vec<Object>,
        update: Vec<Object>,
    },
    ListQueryFilter {
        queries: Vec<Object>,
    },
    AttachmentRecordFields {
        records: Vec<RecordEntry>,
    },
    FileProviderDefaults(Box<DefaultsInput>),
}

pub(crate) struct Op;

const OPERATIONS: [(&str, FileOperation); 5] = [
    ("create", FileOperation::Create),
    ("list", FileOperation::List),
    ("read", FileOperation::Read),
    ("update", FileOperation::Update),
    ("delete", FileOperation::Delete),
];

fn filtered(kind: FileParamsKind, params: &[Object]) -> Value {
    Value::Array(
        params
            .iter()
            .map(|entry| Value::Object(filter_file_params(kind, entry)))
            .collect(),
    )
}

fn decode_record(entry: &RecordEntry) -> crate::helpers::HelperResult {
    let kind = match entry.kind {
        RecordName::FileObject => VectorFileRecordKind::FileObject,
        RecordName::CreateRequest => VectorFileRecordKind::CreateRequest,
        RecordName::UpdateRequest => VectorFileRecordKind::UpdateRequest,
        RecordName::ListQuery => VectorFileRecordKind::ListQuery,
        RecordName::ListResponse => VectorFileRecordKind::ListResponse,
        RecordName::DeleteResponse => VectorFileRecordKind::DeleteResponse,
        RecordName::ContentResponse => VectorFileRecordKind::ContentResponse,
        RecordName::ContentTextPart => VectorFileRecordKind::ContentTextPart,
        RecordName::AuthCredentials => VectorFileRecordKind::AuthCredentials,
        RecordName::ChunkingStrategy => VectorFileRecordKind::ChunkingStrategy,
        RecordName::StaticChunking => VectorFileRecordKind::StaticChunking,
    };
    let bytes =
        serde_json::to_vec(&entry.value).map_err(|error| validation_error(error.to_string()))?;
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_items: 10_000,
    };
    let record = decode_vector_file_contracts(kind, &bytes, limits)
        .map_err(|error| validation_error(error.to_string()))?;
    serde_json::to_value(record).map_err(|error| validation_error(error.to_string()))
}

fn param_reports(input: &DefaultsInput) -> (Object, Object) {
    let supported = OPERATIONS
        .iter()
        .map(|(name, operation)| {
            let params = MinimalConfig.supported_params(*operation).0;
            ((*name).to_owned(), json!(params))
        })
        .collect();
    let mapped = [("True", true), ("False", false)]
        .into_iter()
        .map(|(name, drop_params)| {
            let params = MinimalConfig.map_params(
                FileOperation::List,
                &input.non_default_params,
                input.optional_params.clone(),
                drop_params,
            );
            (name.to_owned(), Value::Object(params))
        })
        .collect();
    (supported, mapped)
}

fn provider_defaults(input: &DefaultsInput) -> Value {
    let config = MinimalConfig;
    let (supported, mapped) = param_reports(input);
    let urls: Vec<Value> = input
        .api_bases
        .iter()
        .map(
            |api_base| match config.complete_url(api_base.as_deref(), "vs_a", &Object::new()) {
                Ok(url) => Value::String(url),
                Err(error) => {
                    json!({"error": {"class": "ValueError", "message": error.to_string()}})
                }
            },
        )
        .collect();
    let api_key = input.api_key.as_deref().map(SecretString::from);
    let (signed_headers, signed_body) = config.sign_request(SignRequest {
        headers: &input.headers,
        optional_params: &input.optional_params,
        request_data: &input.request_data,
        api_base: &input.api_base,
        api_key: api_key.as_ref(),
    });
    let raised = config.error_class(
        &input.error.message,
        input.error.status_code,
        input.error.headers.clone(),
    );
    json!({
        "supported": supported,
        "mapped": mapped,
        "environment": config.validate_environment(&input.headers, None).0,
        "urls": urls,
        "signed_headers": signed_headers,
        "signed_body": signed_body,
        "error": {
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": raised.headers,
        },
    })
}

fn validation_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("ValidationError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::AttachmentParameterFilters { create, update } => Ok(json!({
                "create": filtered(FileParamsKind::Create, create),
                "update": filtered(FileParamsKind::Update, update),
            })),
            Input::ListQueryFilter { queries } => Ok(filtered(FileParamsKind::List, queries)),
            Input::AttachmentRecordFields { records } => records
                .iter()
                .map(decode_record)
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Input::FileProviderDefaults(defaults) => Ok(provider_defaults(defaults)),
        }
    }
}
