use std::collections::BTreeMap;
use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::sections::Section;
use betterlite_config::sections::file_upload_validation::Raw;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_upload_validation::batch::{
    BatchValidationFailure, check_batch_file_upload,
};
use betterlite_core::api::file_upload_validation::failure::FailureFields;
use betterlite_core::api::file_upload_validation::line_shape::LineShape;
use betterlite_core::api::file_upload_validation::source::FileBody;
use betterlite_core::api::file_upload_validation::upload::{
    UploadValidationFailure, check_allowed_extension, check_blocked_extension,
    check_unsafe_filename, check_upload_file_size,
};
use betterlite_error::proxy_exception::ProxyException;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

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

fn failure_json(fields: &FailureFields, exception: &ProxyException) -> Value {
    json!({
        "message": exception.message,
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
        "status": fields.status,
    })
}

fn upload_failure(failure: Option<&UploadValidationFailure>) -> Value {
    match failure {
        None => Value::Null,
        Some(failure) => {
            let fields = failure.fields();
            let exception = failure.exception();
            failure_json(&fields, &exception)
        }
    }
}

fn batch_failure(failure: Option<&BatchValidationFailure>) -> Value {
    match failure {
        None => Value::Null,
        Some(failure) => {
            let fields = failure.fields();
            let exception = failure.exception();
            failure_json(&fields, &exception)
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

fn config_source(
    max_file_size_mb: &Value,
    max_batch_file_size_mb: &Value,
    allowed_file_extensions: &Value,
    blocked_file_extensions: &Value,
) -> Value {
    json!({
        "general_settings": {
            "max_file_size_mb": max_file_size_mb,
            "max_batch_file_size_mb": max_batch_file_size_mb,
            "allowed_file_extensions": allowed_file_extensions,
            "blocked_file_extensions": blocked_file_extensions,
        },
    })
}

fn settings_answer(
    max_file_size_mb: &Value,
    max_batch_file_size_mb: &Value,
    allowed_file_extensions: &Value,
    blocked_file_extensions: &Value,
) -> Value {
    let source = config_source(
        max_file_size_mb,
        max_batch_file_size_mb,
        allowed_file_extensions,
        blocked_file_extensions,
    );
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
            Ok(json!({ "failure": upload_failure(failure.as_ref()) }))
        }
        Call::Extensions {
            filename,
            allowed,
            blocked,
        } => {
            let allowed = check_allowed_extension(filename.as_deref(), allowed.as_deref());
            let blocked = check_blocked_extension(filename.as_deref(), blocked.as_deref());
            Ok(json!({
                "allowed": upload_failure(allowed.as_ref()),
                "blocked": upload_failure(blocked.as_ref()),
            }))
        }
        Call::UploadSize {
            source,
            max_file_size_mb,
        } => {
            let data = decode_source(source)?;
            match source {
                Source::Bytes { .. } => {
                    let mut body = FileBody::Bytes(&data);
                    let failure = check_upload_file_size(&mut body, *max_file_size_mb)
                        .map_err(|error| value_error(&error))?;
                    Ok(json!({
                        "failure": upload_failure(failure.as_ref()),
                        "position": Value::Null,
                    }))
                }
                Source::Seekable { position, .. } => {
                    let mut cursor = Cursor::new(data);
                    cursor.set_position(*position);
                    let outcome = {
                        let mut body = FileBody::Seekable(&mut cursor);
                        check_upload_file_size(&mut body, *max_file_size_mb)
                    };
                    let failure = outcome.map_err(|error| value_error(&error))?;
                    Ok(json!({
                        "failure": upload_failure(failure.as_ref()),
                        "position": cursor.position(),
                    }))
                }
            }
        }
        Call::Batch {
            filename,
            source,
            max_batch_file_size_mb,
            line_shape,
        } => {
            let data = decode_source(source)?;
            let shape = line_shape.shape();
            match source {
                Source::Bytes { .. } => {
                    let mut body = FileBody::Bytes(&data);
                    let failure = check_batch_file_upload(
                        filename.as_deref(),
                        &mut body,
                        *max_batch_file_size_mb,
                        shape,
                        ApiLimits::default(),
                    )
                    .map_err(|error| value_error(&error))?;
                    Ok(json!({
                        "failure": batch_failure(failure.as_ref()),
                        "position": Value::Null,
                    }))
                }
                Source::Seekable { position, .. } => {
                    let mut cursor = Cursor::new(data);
                    cursor.set_position(*position);
                    let outcome = {
                        let mut body = FileBody::Seekable(&mut cursor);
                        check_batch_file_upload(
                            filename.as_deref(),
                            &mut body,
                            *max_batch_file_size_mb,
                            shape,
                            ApiLimits::default(),
                        )
                    };
                    let failure = outcome.map_err(|error| value_error(&error))?;
                    Ok(json!({
                        "failure": batch_failure(failure.as_ref()),
                        "position": cursor.position(),
                    }))
                }
            }
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
