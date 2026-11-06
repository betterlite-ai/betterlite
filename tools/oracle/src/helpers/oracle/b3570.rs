use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, RelayCallType, RelayShape,
};
use betterlite_openai::passthrough::transformation::{
    logged_relay_shape, model_group_from, relayed_body, relayed_json_object, replace_path_segment,
    strip_leading_model_segment,
};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ModelGroup {
        #[serde(default)]
        litellm_params: Object,
    },
    StripLeadingModelSegment {
        endpoint: String,
        model_names: Vec<String>,
    },
    ReplacePathSegment {
        endpoint: String,
        segment: String,
        replacement: String,
    },
    RelayedJsonObject {
        status_code: u16,
        body: String,
    },
    RelayedBody {
        body: String,
    },
    LoggedRelayShape(RelayRequest),
}

#[derive(Debug, Deserialize)]
pub(crate) struct RelayRequest {
    endpoint: String,
    status_code: u16,
    body: String,
    shapes: Vec<ShapeInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShapeInput {
    path_suffix: String,
    call_type: CallTypeInput,
    parse: ParseInput,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CallTypeInput {
    Aembedding,
    AimageGeneration,
    Arerank,
    Aresponses,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ParseInput {
    Mapping,
    Fail,
}

struct Logging {
    call_type: Option<RelayCallType>,
}

impl PassthroughLogging for Logging {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.call_type = Some(call_type);
    }
}

fn failure(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

const fn call_type(call_type: CallTypeInput) -> RelayCallType {
    match call_type {
        CallTypeInput::Aembedding => RelayCallType::Aembedding,
        CallTypeInput::AimageGeneration => RelayCallType::AimageGeneration,
        CallTypeInput::Arerank => RelayCallType::Arerank,
        CallTypeInput::Aresponses => RelayCallType::Aresponses,
    }
}

fn parser(parse: ParseInput) -> fn(&Object) -> Option<Object> {
    match parse {
        ParseInput::Mapping => |body| Some(body.clone()),
        ParseInput::Fail => |_body| None,
    }
}

fn to_shape(shape: &ShapeInput) -> RelayShape<'_, Object> {
    RelayShape {
        path_suffix: &shape.path_suffix,
        call_type: call_type(shape.call_type),
        parse: parser(shape.parse),
    }
}

fn relay(request: &RelayRequest) -> serde_json::Value {
    let shapes: Vec<RelayShape<'_, Object>> = request.shapes.iter().map(to_shape).collect();
    let mut logging = Logging { call_type: None };
    let relayed = logged_relay_shape(
        &shapes,
        request.status_code,
        request.body.as_bytes(),
        &mut logging,
        &request.endpoint,
    );
    json!({
        "call_type": logging.call_type.map(RelayCallType::as_str),
        "relayed": relayed,
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ModelGroup { litellm_params } => {
                Ok(json!({"model_group": model_group_from(litellm_params)}))
            }
            Input::StripLeadingModelSegment {
                endpoint,
                model_names,
            } => Ok(json!({
                "endpoint": strip_leading_model_segment(endpoint, model_names)
            })),
            Input::ReplacePathSegment {
                endpoint,
                segment,
                replacement,
            } => Ok(json!({
                "endpoint": replace_path_segment(endpoint, segment, replacement)
            })),
            Input::RelayedJsonObject { status_code, body } => Ok(json!({
                "relayed": relayed_json_object(*status_code, body.as_bytes())
            })),
            Input::RelayedBody { body } => {
                let relayed = relayed_body(body.as_bytes())
                    .map_err(|error| failure("PassthroughError", &error.to_string()))?;
                let value = serde_json::to_value(relayed)
                    .map_err(|error| failure("SerializationError", &error.to_string()))?;
                Ok(json!({"relayed": value}))
            }
            Input::LoggedRelayShape(request) => Ok(relay(request)),
        }
    }
}
