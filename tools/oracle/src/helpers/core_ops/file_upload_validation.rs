use crate::helpers::{Helper, HelperError, HelperResult};
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
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Cursor;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    UnsafeFilename {
        filename: Option<String>,
    },
    Extensions {
        filename: Option<String>,
        allowed: Option<Vec<String>>,
        blocked: Option<Vec<String>>,
    },
    UploadSize {
        source: Source,
        max_file_size_mb: Option<i64>,
    },
    Batch {
        filename: Option<String>,
        source: Source,
        max_batch_file_size_mb: Option<i64>,
        line_shape: ShapeName,
    },
    Settings {
        max_file_size_mb: Value,
        max_batch_file_size_mb: Value,
        allowed_file_extensions: Value,
        blocked_file_extensions: Value,
    },
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
fn value_error(error: &dyn std::fmt::Display) -> Box<HelperError> {
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
    let Some(fields) = fields else {
        return Value::Null;
    };
    let status = fields.status;
    let exception = exception(fields);
    json!({
        "message": exception.message,
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
        "status": status,
    })
}
fn with_source<T, F>(source: &Source, check: F) -> Result<(T, Value), Box<HelperError>>
where
    F: FnOnce(&mut FileBody<'_>) -> Result<T, FileUploadValidationError>,
{
    let data = decode_source(source)?;
    match source {
        Source::Bytes { .. } => {
            let outcome =
                check(&mut FileBody::Bytes(&data)).map_err(|error| value_error(&error))?;
            Ok((outcome, Value::Null))
        }
        Source::Seekable { position, .. } => {
            let mut cursor = Cursor::new(data);
            cursor.set_position(*position);
            let outcome =
                check(&mut FileBody::Seekable(&mut cursor)).map_err(|error| value_error(&error))?;
            Ok((outcome, json!(cursor.position())))
        }
    }
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
fn settings_answer(
    max_file_size_mb: &Value,
    max_batch_file_size_mb: &Value,
    allowed_file_extensions: &Value,
    blocked_file_extensions: &Value,
) -> Value {
    let source = json!({
        "general_settings": {
            "max_file_size_mb": max_file_size_mb,
            "max_batch_file_size_mb": max_batch_file_size_mb,
            "allowed_file_extensions": allowed_file_extensions,
            "blocked_file_extensions": blocked_file_extensions,
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
        Call::UnsafeFilename { filename } => {
            let failure = check_unsafe_filename(filename.as_deref());
            Ok(
                json!({ "failure": failure_json(failure.as_ref().map(UploadValidationFailure::fields)) }),
            )
        }
        Call::Extensions {
            filename,
            allowed,
            blocked,
        } => {
            let allowed = check_extension(
                filename.as_deref(),
                allowed.as_deref(),
                ExtensionRule::Allowed,
            );
            let blocked = check_extension(
                filename.as_deref(),
                blocked.as_deref(),
                ExtensionRule::Blocked,
            );
            Ok(json!({
                "allowed": failure_json(allowed.as_ref().map(UploadValidationFailure::fields)),
                "blocked": failure_json(blocked.as_ref().map(UploadValidationFailure::fields)),
            }))
        }
        Call::UploadSize {
            source,
            max_file_size_mb,
        } => {
            let (failure, position) = with_source(source, |body| {
                check_upload_file_size(body, *max_file_size_mb)
            })?;
            Ok(json!({
                "failure": failure_json(failure.as_ref().map(UploadValidationFailure::fields)),
                "position": position,
            }))
        }
        Call::Batch {
            filename,
            source,
            max_batch_file_size_mb,
            line_shape,
        } => {
            let (failure, position) = with_source(source, |body| {
                check_batch_file_upload(
                    filename.as_deref(),
                    body,
                    *max_batch_file_size_mb,
                    line_shape.shape(),
                    ApiLimits::default(),
                )
            })?;
            Ok(json!({
                "failure": failure_json(failure.as_ref().map(BatchValidationFailure::fields)),
                "position": position,
            }))
        }
        Call::Settings {
            max_file_size_mb,
            max_batch_file_size_mb,
            allowed_file_extensions,
            blocked_file_extensions,
        } => Ok(settings_answer(
            max_file_size_mb,
            max_batch_file_size_mb,
            allowed_file_extensions,
            blocked_file_extensions,
        )),
    }
}
