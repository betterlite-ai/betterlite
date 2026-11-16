use std::collections::BTreeMap;

use betterlite_core::api::stability::generation::StabilityImageGenerationConfig;
use betterlite_core::api::stability::generation::reply::decode_stability_image;
// ast-grep-ignore: no-long-names
use betterlite_core::api::stability::generation::get_stability_image_generation_config as generation_config;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Supported {
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
    Url {
        model: String,
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Environment {
        model: String,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        headers: ImageHeaders,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Request {
        prompt: String,
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
        Call::Supported { model } => Ok(supported_answer(model)),
        Call::Map {
            model,
            non_default_params: params,
            optional_params: existing,
            drop_params,
        } => Ok(map_answer(model, params, existing, *drop_params)),
        Call::Url {
            model,
            api_base,
            environment,
        } => Ok(url_answer(model, api_base.as_deref(), environment)),
        Call::Environment {
            model,
            api_key,
            headers,
            environment,
        } => Ok(environment_answer(
            model,
            api_key.as_deref(),
            headers,
            environment,
        )),
        Call::Request {
            prompt,
            optional_params,
        } => Ok(request_answer(prompt, optional_params)),
        Call::Response {
            status,
            headers,
            body,
            created,
            data,
        } => response_answer(*status, headers, body, *created, data),
    }
}

// LiteLLM's get_supported_openai_params answers the same three names for every model
fn supported_answer(_model: &str) -> Value {
    Value::Array(
        StabilityImageGenerationConfig::SUPPORTED_OPENAI_PARAMS
            .iter()
            .map(|name| Value::String((*name).to_owned()))
            .collect(),
    )
}

fn map_answer(
    model: &str,
    non_default_params: &ImageParams,
    optional_params: &ImageParams,
    drop_params: bool,
) -> Value {
    let attempt = StabilityImageGenerationConfig::map_openai_params(
        non_default_params,
        optional_params.clone(),
        model,
        drop_params,
    );
    if let Ok(mapped) = attempt {
        return json!({"mapped": mapped});
    }
    json!({"unsupported": true})
}

fn url_answer(
    model: &str,
    api_base: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Value {
    let config = generation_config(model, environment);
    json!({
        "url": config.get_complete_url(api_base, model)
    })
}

fn environment_answer(
    model: &str,
    api_key: Option<&str>,
    headers: &ImageHeaders,
    environment: &BTreeMap<String, String>,
) -> Value {
    let config = generation_config(model, environment);
    let explicit = api_key.map(SecretString::from);
    if let Ok(validated) = config.validate_environment(headers.clone(), explicit.as_ref()) {
        return json!({"headers": validated});
    }
    json!({"missing_key": true})
}

fn request_answer(prompt: &str, optional_params: &ImageParams) -> Value {
    let attempt =
        // ast-grep-ignore: no-long-names
        StabilityImageGenerationConfig::transform_image_generation_request(prompt, optional_params);
    if let Ok(request) = attempt {
        return json!({"request": request});
    }
    json!({"invalid": true})
}

fn response_answer(
    status: u16,
    headers: &ImageHeaders,
    body: &str,
    created: i64,
    data: &[ImageData],
) -> Result<Value, Box<HelperError>> {
    let upstream = ImageHttpResponse {
        status_code: status,
        headers: headers.clone(),
        body: body.as_bytes().to_vec(),
    };
    let mut response = ImageResponse {
        created,
        data: (!data.is_empty()).then(|| data.to_vec()),
        usage: None,
    };
    match decode_stability_image(&upstream, &mut response) {
        Ok(()) => Ok(json!({"response": response})),
        Err(error) => Err(Box::new(HelperError {
            class: "BaseLLMException".to_owned(),
            status: Some(error.status()),
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}
