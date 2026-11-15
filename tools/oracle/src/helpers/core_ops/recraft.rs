use std::collections::BTreeMap;

use betterlite_models::tiered_pricing::Scalar;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageHeaders, ImageParams};
use serde::Deserialize;
use serde_json::Value;

mod answers;

use crate::helpers::{Helper, HelperError, HelperResult};

use answers::{
    cost_answer, edit_map_answer, edit_request_answer, edit_response_answer, environment_answer,
    map_answer, request_answer, response_answer, supported_answer, url_answer,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Endpoint {
    Generation,
    Edit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageSeed {
    #[serde(default)]
    filename: Option<String>,
    data_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Supported {
        endpoint: Endpoint,
        model: String,
    },
    Map {
        model: String,
        #[serde(default)]
        non_default_params: ImageParams,
        #[serde(default)]
        optional_params: ImageParams,
        #[serde(default)]
        drop_params: bool,
    },
    EditMap {
        model: String,
        #[serde(default)]
        params: ImageParams,
        #[serde(default)]
        drop_params: bool,
    },
    Url {
        endpoint: Endpoint,
        model: String,
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Environment {
        endpoint: Endpoint,
        model: String,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        headers: ImageHeaders,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Request {
        model: String,
        prompt: String,
        #[serde(default)]
        optional_params: ImageParams,
    },
    EditRequest {
        model: String,
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        images: Vec<ImageSeed>,
        #[serde(default)]
        optional_params: ImageParams,
    },
    Response {
        status: u16,
        #[serde(default)]
        headers: ImageHeaders,
        body: String,
        #[serde(default)]
        created: i64,
        #[serde(default)]
        data: Vec<ImageData>,
    },
    EditResponse {
        status: u16,
        #[serde(default)]
        headers: ImageHeaders,
        body: String,
    },
    Cost {
        model: String,
        #[serde(default)]
        output_cost_per_image: Option<Scalar>,
        #[serde(default)]
        image_count: i64,
        #[serde(default)]
        other: bool,
    },
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
        Call::Supported { endpoint, model } => Ok(supported_answer(*endpoint, model)),
        Call::Map {
            model,
            non_default_params,
            optional_params,
            drop_params,
        } => Ok(map_answer(
            model,
            non_default_params,
            optional_params,
            *drop_params,
        )),
        Call::EditMap {
            model,
            params,
            drop_params,
        } => Ok(edit_map_answer(model, params, *drop_params)),
        Call::Url {
            endpoint,
            model,
            api_base,
            environment,
        } => Ok(url_answer(
            *endpoint,
            model,
            api_base.as_deref(),
            environment,
        )),
        Call::Environment {
            endpoint,
            model,
            api_key,
            headers,
            environment,
        } => Ok(environment_answer(
            *endpoint,
            model,
            api_key.as_deref(),
            headers,
            environment,
        )),
        Call::Request {
            model,
            prompt,
            optional_params,
        } => request_answer(model, prompt, optional_params),
        Call::EditRequest {
            model,
            prompt,
            images,
            optional_params,
        } => edit_request_answer(model, prompt.as_deref(), images, optional_params),
        Call::Response {
            status,
            headers,
            body,
            created,
            data,
        } => response_answer(*status, headers, body, *created, data),
        Call::EditResponse {
            status,
            headers,
            body,
        } => edit_response_answer(*status, headers, body),
        Call::Cost {
            model,
            output_cost_per_image,
            image_count,
            other,
        } => cost_answer(model, output_cost_per_image.as_ref(), *image_count, *other),
    }
}
