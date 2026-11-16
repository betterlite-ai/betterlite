use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::sections::Section;
use betterlite_config::sections::file_upload_validation::Raw;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_upload_validation::batch::{
    BatchValidationFailure, check_batch_file_upload,
};
use betterlite_core::api::file_upload_validation::error::FileUploadValidationError;
use betterlite_core::api::file_upload_validation::failure::{FailureFields, exception};
use betterlite_core::api::file_upload_validation::line_shape::LineShape;
use betterlite_core::api::file_upload_validation::source::FileBody;
use betterlite_core::api::file_upload_validation::upload::{
    ExtensionRule, UploadValidationFailure, check_extension, check_unsafe_filename,
    check_upload_file_size,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Cursor;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    UnsafeFilename(UnsafeFilenameCall),
    Extensions(ExtensionsCall),
    UploadSize(UploadSizeCall),
    Batch(BatchCall),
    Settings(SettingsCall),
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnsafeFilenameCall {
    filename: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionsCall {
    filename: Option<String>,
    allowed: Option<Vec<String>>,
    blocked: Option<Vec<String>>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadSizeCall {
    source: Source,
    max_file_size_mb: Option<i64>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchCall {
    filename: Option<String>,
    source: Source,
    max_batch_file_size_mb: Option<i64>,
    line_shape: ShapeName,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsCall {
    max_file_size_mb: Value,
    max_batch_file_size_mb: Value,
    allowed_file_extensions: Value,
    blocked_file_extensions: Value,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Source {
    Bytes {
        data: String,
        repeat: usize,
        pad: usize,
    },
    Seekable {
        data: String,
        repeat: usize,
        pad: usize,
        position: u64,
    },
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShapeName {
    Openai,
    Vertex,
}
impl ShapeName {
    const fn shape(self) -> LineShape {
        match self {
            Self::Openai => LineShape::OpenAi,
            Self::Vertex => LineShape::VertexPassthrough,
        }
    }
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let answers = input
            .calls
            .iter()
            .map(run_call)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(answers))
    }
}
fn value_error(error: &dyn Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
fn failure_json(fields: Option<FailureFields>) -> Value {
    let Some(present) = fields else {
        return Value::Null;
    };
    let status = present.status;
    let exception = exception(present);
    json!({
        "message": exception.message,
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
        "status": status,
    })
}
fn decode_source(source: &Source) -> Result<Vec<u8>, Box<HelperError>> {
    let (data, repeat, pad) = match source {
        Source::Bytes { data, repeat, pad }
        | Source::Seekable {
            data, repeat, pad, ..
        } => (data, *repeat, *pad),
    };
    let chunk = BASE64.decode(data).map_err(|error| value_error(&error))?;
    let mut bytes = chunk.repeat(repeat);
    bytes.resize(bytes.len().saturating_add(pad), b' ');
    Ok(bytes)
}
fn source_answer<T, F>(
    source: &Source,
    check: F,
    fields: fn(&T) -> FailureFields,
) -> Result<Value, Box<HelperError>>
where
    F: FnOnce(&mut FileBody<'_>) -> Result<Option<T>, FileUploadValidationError>,
{
    let data = decode_source(source)?;
    let (failure, position) = match source {
        Source::Bytes { .. } => {
            let failure =
                check(&mut FileBody::Bytes(&data)).map_err(|error| value_error(&error))?;
            (failure, Value::Null)
        }
        Source::Seekable { position, .. } => {
            let mut cursor = Cursor::new(data);
            cursor.set_position(*position);
            let failure =
                check(&mut FileBody::Seekable(&mut cursor)).map_err(|error| value_error(&error))?;
            (failure, json!(cursor.position()))
        }
    };
    Ok(json!({
        "failure": failure_json(failure.as_ref().map(fields)),
        "position": position,
    }))
}
fn extensions_answer(call: &ExtensionsCall) -> Value {
    let allow_failure = check_extension(
        call.filename.as_deref(),
        call.allowed.as_deref(),
        ExtensionRule::Allowed,
    );
    let block_failure = check_extension(
        call.filename.as_deref(),
        call.blocked.as_deref(),
        ExtensionRule::Blocked,
    );
    json!({
        "allowed": failure_json(allow_failure.as_ref().map(UploadValidationFailure::fields)),
        "blocked": failure_json(block_failure.as_ref().map(UploadValidationFailure::fields)),
    })
}
fn settings_answer(call: &SettingsCall) -> Value {
    let source = json!({
        "general_settings": {
            "max_file_size_mb": call.max_file_size_mb,
            "max_batch_file_size_mb": call.max_batch_file_size_mb,
            "allowed_file_extensions": call.allowed_file_extensions,
            "blocked_file_extensions": call.blocked_file_extensions,
        },
    });
    let Ok(raw) = serde_json::from_value::<Raw>(source) else {
        return json!({ "error": "invalid_setting" });
    };
    let lookup = BTreeMap::<String, String>::new();
    match Section::convert(raw, &lookup) {
        Ok(settings) => json!({
            "settings": {
                "max_file_size_mb": settings.max_file_size_mb,
                "max_batch_file_size_mb": settings.max_batch_file_size_mb,
                "allowed_file_extensions": settings.allowed_file_extensions,
                "blocked_file_extensions": settings.blocked_file_extensions,
            },
        }),
        Err(_error) => json!({ "error": "invalid_setting" }),
    }
}
fn run_call(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::UnsafeFilename(payload) => {
            let failure = check_unsafe_filename(payload.filename.as_deref());
            Ok(json!({
                "failure": failure_json(failure.as_ref().map(UploadValidationFailure::fields)),
            }))
        }
        Call::Extensions(payload) => Ok(extensions_answer(payload)),
        Call::UploadSize(payload) => source_answer(
            &payload.source,
            |body| check_upload_file_size(body, payload.max_file_size_mb),
            UploadValidationFailure::fields,
        ),
        Call::Batch(payload) => source_answer(
            &payload.source,
            |body| {
                check_batch_file_upload(
                    payload.filename.as_deref(),
                    body,
                    payload.max_batch_file_size_mb,
                    payload.line_shape.shape(),
                    ApiLimits::default(),
                )
            },
            BatchValidationFailure::fields,
        ),
        Call::Settings(payload) => Ok(settings_answer(payload)),
    }
}
