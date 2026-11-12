use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_core::api::image_input_options::mime::{
    DEFAULT_MIME, image_content_type_seekable, image_content_type_stream, image_type_from_name,
    mime_for_image_type,
};
use betterlite_core::api::image_input_options::options::ImageEditOptionalRequestParams;
use betterlite_core::api::image_input_options::selection::{
    ImageEditProviderConfig, ImageInputOptionsError, OptionalParamsPolicy,
    UNSUPPORTED_PARAMS_STATUS, optional_params_image_edit, requested_optional_params,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod readers;

use readers::{Failure, SeekableReader, StreamReader};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Select {
        supported: Vec<String>,
        params: ImageEditOptionalRequestParams,
    },
    Optional {
        model: String,
        supported: Vec<String>,
        global_drop_params: bool,
        call_drop_params: Option<bool>,
        additional_drop_params: Option<Vec<String>>,
        params: ImageEditOptionalRequestParams,
    },
    Content {
        fixture: Fixture,
    },
    Detected {
        detection: Option<String>,
        raises: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    kind: FixtureKind,
    data_base64: String,
    position: usize,
    name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FixtureKind {
    Bytes,
    BytesIo,
    BufferedReader,
    Seekable,
    Nonseekable,
    NoRead,
    FailTell,
    FailSeek,
    FailRead,
    FailRestore,
    NonBytesRead,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let values = input
            .calls
            .iter()
            .map(run_call)
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(Value::Array(values))
    }
}

fn run_call(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Select { supported, params } => {
            let selected = requested_optional_params(params, supported);
            Ok(json!(selected))
        }
        Call::Optional {
            model,
            supported,
            global_drop_params,
            call_drop_params,
            additional_drop_params,
            params,
        } => {
            let provider = RecordingProvider {
                supported: supported.clone(),
            };
            let policy = OptionalParamsPolicy {
                global_drop_params: *global_drop_params,
                call_drop_params: call_drop_params.unwrap_or(false),
                additional_drop_params: additional_drop_params.clone().unwrap_or_default(),
            };
            match optional_params_image_edit(&provider, model, params, &policy) {
                Ok(mapped) => Ok(json!({"ok": true, "value": mapped})),
                Err(ImageInputOptionsError::UnsupportedParams { .. }) => {
                    Ok(json!({"ok": false, "status": UNSUPPORTED_PARAMS_STATUS}))
                }
            }
        }
        Call::Content { fixture } => content_call(fixture),
        Call::Detected { detection, raises } => {
            let image_type = if *raises {
                None
            } else {
                detection.as_deref().and_then(image_type_from_name)
            };
            Ok(json!({"mime": mime_for_image_type(image_type)}))
        }
    }
}

fn content_call(fixture: &Fixture) -> Result<Value, Box<HelperError>> {
    let data = BASE64.decode(&fixture.data_base64).map_err(|error| {
        Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    let name = fixture.name.clone();
    if fixture.kind == FixtureKind::Bytes {
        return Ok(json!({"mime": image_content_type_stream(&mut data.as_slice()), "name": name}));
    }
    if fixture.kind == FixtureKind::NoRead {
        return Ok(json!({"mime": DEFAULT_MIME, "name": name}));
    }
    let failure = failure_of(fixture.kind);
    if fixture.kind == FixtureKind::Nonseekable {
        let mut reader = StreamReader::new(data, fixture.position, failure);
        let mime = image_content_type_stream(&mut reader);
        return Ok(json!({
            "mime": mime,
            "name": name,
            "position": reader.position,
        }));
    }
    let mut reader = SeekableReader::new(data, fixture.position, failure);
    let mime = image_content_type_seekable(&mut reader);
    Ok(json!({
        "mime": mime,
        "name": name,
        "position": reader.cursor.position(),
    }))
}

const fn failure_of(kind: FixtureKind) -> Failure {
    match kind {
        FixtureKind::FailTell => Failure::Tell,
        FixtureKind::FailSeek => Failure::Seek,
        FixtureKind::FailRead => Failure::Read,
        FixtureKind::FailRestore => Failure::Restore,
        FixtureKind::NonBytesRead => Failure::NonBytes,
        FixtureKind::Bytes
        | FixtureKind::BytesIo
        | FixtureKind::BufferedReader
        | FixtureKind::Seekable
        | FixtureKind::Nonseekable
        | FixtureKind::NoRead => Failure::None,
    }
}

struct RecordingProvider {
    supported: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MappedCall {
    model: String,
    drop_params: bool,
    params: ImageEditOptionalRequestParams,
}

impl ImageEditProviderConfig for RecordingProvider {
    type Error = ImageInputOptionsError;
    type Mapped = MappedCall;

    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.clone()
    }

    fn map_openai_params(
        &self,
        options: &ImageEditOptionalRequestParams,
        model: &str,
        drop_params: bool,
    ) -> Result<MappedCall, ImageInputOptionsError> {
        Ok(MappedCall {
            model: model.to_owned(),
            drop_params,
            params: options.clone(),
        })
    }
}
