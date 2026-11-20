use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::topaz::MODELS;
use betterlite_core::api::topaz::params::{SUPPORTED_OPENAI_PARAMS, get_complete_url};
use betterlite_core::api::topaz::reply::{TopazTiming, transform_response};
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod answers;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Models,
    BaseModel {
        model: String,
    },
    Supported,
    Credential {
        field: CredentialField,
        // the explicit value the caller passes, absent when only the environment resolves it
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Environment {
        #[serde(default)]
        api_key: Option<String>,
        // the source ignores the headers a caller passes; the case still supplies them to prove it
        headers: ImageHeaders,
    },
    Url {
        #[serde(default)]
        api_base: Option<String>,
    },
    Map {
        #[serde(default)]
        non_default_params: ImageParams,
        #[serde(default)]
        optional_params: ImageParams,
        // map_openai_params ignores drop_params for every key it does not map
        #[serde(default, rename = "drop_params")]
        _drop_params: bool,
    },
    Upload {
        file: FileSpec,
        // absent when the call only prepares the file tuple
        #[serde(default)]
        optional_params: Option<ImageParams>,
    },
    ResponseSync {
        body: Vec<u8>,
        elapsed_ms: u64,
        created: i64,
    },
    ResponseAsync {
        body: Vec<u8>,
        response_ms: f64,
        created: i64,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CredentialField {
    ApiKey,
    ApiBase,
}

// prepare_file_tuple's input, with the members the source reads from each shape
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSpec {
    kind: FileKind,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    headers: ImageHeaders,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FileKind {
    Bytes,
    Tuple2,
    Tuple3,
    Tuple4,
    Other,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(Value::Array(input.calls.iter().map(run_call).collect()))
    }
}

fn run_call(call: &Call) -> Value {
    match call {
        Call::Models => json!(MODELS),
        Call::BaseModel { model } => json!(model),
        Call::Supported => json!(SUPPORTED_OPENAI_PARAMS),
        Call::Credential {
            field,
            value,
            environment,
        } => answers::credential_answer(*field, value.as_deref(), environment),
        Call::Environment {
            api_key,
            headers: _headers,
        } => answers::environment_answer(api_key.as_deref()),
        Call::Url { api_base } => json!({"url": get_complete_url(api_base.as_deref())}),
        Call::Map {
            non_default_params,
            optional_params,
            _drop_params,
        } => answers::map_answer(non_default_params, optional_params),
        Call::Upload {
            file,
            optional_params,
        } => answers::upload_answer(file, optional_params.as_ref()),
        // the sync entry point reads the raw response's elapsed time
        Call::ResponseSync {
            body,
            elapsed_ms,
            created,
        } => {
            let timing = TopazTiming::Elapsed(Duration::from_millis(*elapsed_ms));
            answers::reply_answer(&transform_response(body, timing, *created), "elapsed")
        }
        // the async entry point reads the call's recorded response time instead
        Call::ResponseAsync {
            body,
            response_ms,
            created,
        } => {
            let timing = TopazTiming::Measured(*response_ms);
            answers::reply_answer(&transform_response(body, timing, *created), "measured")
        }
    }
}
