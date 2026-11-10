use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::codestral_records::RecordSource;
use betterlite_openai::codestral_records::chunk::decode_codestral_chunk;
use betterlite_openai::codestral_records::error::TextCompletionCodestralError;
use betterlite_openai::codestral_records::options::{
    CodestralSettings, SUPPORTED_PARAMS, map_params,
};
use betterlite_openai::codestral_records::reply::{PostCall, PostCallLogger, process_response};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    settings: CodestralSettings,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Options {
        model: String,
        params: Vec<(String, Value)>,
        #[serde(default)]
        optional_params: Map<String, Value>,
    },
    Reply {
        status: u16,
        body: String,
        #[serde(default)]
        messages: Vec<Value>,
        #[serde(default)]
        api_key: String,
        #[serde(default)]
        request_data: Value,
    },
    Chunk {
        line: String,
    },
}

struct Recorder(Vec<Value>);

impl PostCallLogger for Recorder {
    fn post_call(&mut self, call: PostCall<'_>) {
        self.0.push(json!({
            "name": "post_call",
            "input": call.input,
            "api_key": call.api_key,
            "original_response": call.original_response,
            "complete_input_dict": call.request_data,
        }));
    }
}

fn invalid(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn reply(
    status: u16,
    call: PostCall<'_>,
    source: &RecordSource,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let mut recorder = Recorder(Vec::new());
    let outcome = match process_response(status, call, &mut recorder, source) {
        Ok(reply) => json!({"kind": "reply", "reply": reply}),
        Err(TextCompletionCodestralError::Native(native)) => json!({
            "kind": "error",
            "status_code": native.status_code,
            "request": {"method": native.request.method, "url": native.request.url},
            "response_status": native.response.status_code,
        }),
        Err(other) => return Err(invalid(other.to_string())),
    };
    Ok(json!({"events": recorder.0, "outcome": outcome}))
}

fn execute(
    step: &Step,
    config: &Map<String, Value>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let source = RecordSource {
        id: Uuid::nil(),
        created: 0,
    };
    match step {
        Step::Options {
            model,
            params,
            optional_params,
        } => {
            let mut mapped = optional_params.clone();
            map_params(params, &mut mapped);
            Ok(json!({
                "model": model,
                "config": config,
                "supported": SUPPORTED_PARAMS,
                "mapped": mapped,
            }))
        }
        Step::Reply {
            status,
            body,
            messages,
            api_key,
            request_data,
        } => {
            let call = PostCall {
                input: messages,
                api_key,
                original_response: body,
                request_data,
            };
            reply(*status, call, &source)
        }
        Step::Chunk { line } => {
            let chunk = decode_codestral_chunk(line.as_bytes(), &source)
                .map_err(|error| invalid(error.to_string()))?;
            serde_json::to_value(chunk).map_err(|error| invalid(error.to_string()))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let config =
            provider_config(&input.settings).map_err(|error| invalid(error.to_string()))?;
        input
            .steps
            .iter()
            .map(|step| execute(step, &config))
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}
