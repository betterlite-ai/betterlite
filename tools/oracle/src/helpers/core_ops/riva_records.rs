use betterlite_core::api::riva::records::grpc_status::{
    CodeAttribute, DetailsAttribute, GrpcErrorObservation, grpc_error_to_exception,
};
use betterlite_core::api::riva::records::options::map_openai_params;
use betterlite_core::api::riva::records::recognition::transform_request;
use betterlite_core::api::riva::records::response::{FinalResult, build_transcription_response};
use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;
use serde_json::{Value, json};

type Failure = Box<crate::helpers::HelperError>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    #[serde(default)]
    params: Object,
    #[serde(default)]
    final_results: Vec<FinalResult>,
    #[serde(default)]
    response_format: String,
    #[serde(default)]
    duration_seconds: Option<f64>,
    #[serde(default)]
    timestamp_granularities: Option<Vec<String>>,
    #[serde(default)]
    error: Option<ErrorSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorSpec {
    code: CodeAttribute,
    details: DetailsAttribute,
    text: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    LanguageCases,
    TimingFlags,
    EndpointingPrecedence,
    EmptyModel,
    ConcatenatedFinals,
    VerboseWords,
    GrpcErrorTable,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes = input
            .calls
            .iter()
            .map(run_call)
            .collect::<Result<Vec<_>, Failure>>()?;
        Ok(Value::Array(outcomes))
    }
}

fn run_call(call: &Call) -> Result<Value, Failure> {
    match call.function {
        Function::LanguageCases
        | Function::TimingFlags
        | Function::EndpointingPrecedence
        | Function::EmptyModel => request_payload(call),
        Function::ConcatenatedFinals | Function::VerboseWords => {
            let built = build_transcription_response(
                &call.final_results,
                &call.response_format,
                call.duration_seconds,
                call.timestamp_granularities.as_deref(),
            )
            .map_err(failure)?;
            serde_json::to_value(built).map_err(failure)
        }
        Function::GrpcErrorTable => {
            let Some(spec) = &call.error else {
                return Err(failure("error is required"));
            };
            let observation = GrpcErrorObservation {
                code: spec.code.clone(),
                details: spec.details.clone(),
                text: spec.text.clone(),
            };
            let exception = grpc_error_to_exception(&observation);
            Ok(json!({"status": exception.status_code, "message": exception.message}))
        }
    }
}

fn request_payload(call: &Call) -> Result<Value, Failure> {
    let optional_params = map_openai_params(&call.params).map_err(failure)?;
    let request = transform_request(&optional_params).map_err(failure)?;
    let data = request.request_data().map_err(failure)?;
    serde_json::to_value(data.data).map_err(failure)
}

fn failure<Error>(error: Error) -> Failure
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
