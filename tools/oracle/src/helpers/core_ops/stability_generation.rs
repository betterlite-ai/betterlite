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

use crate::helpers::{Helper, HelperError, HelperResult};

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
        Call::Supported { .. } => Ok(json!(
            StabilityImageGenerationConfig::SUPPORTED_OPENAI_PARAMS
        )),
        Call::Map {
            model,
            non_default_params,
            optional_params,
            drop_params,
        } => {
            let mapped = StabilityImageGenerationConfig::map_openai_params(
                non_default_params,
                optional_params.clone(),
                model,
                *drop_params,
            );
            match mapped {
                Ok(mapped) => Ok(json!({"mapped": mapped})),
                Err(_failure) => Ok(json!({"unsupported": true})),
            }
        }
        Call::Url {
            model,
            api_base,
            environment,
        } => {
            let config = generation_config(model, environment);
            Ok(json!({"url": config.get_complete_url(api_base.as_deref(), model)}))
        }
        Call::Environment {
            model,
            api_key,
            headers,
            environment,
        } => {
            let config = generation_config(model, environment);
            let explicit = api_key.as_ref().map(|key| SecretString::from(key.clone()));
            match config.validate_environment(headers.clone(), explicit.as_ref()) {
                Ok(headers) => Ok(json!({"headers": headers})),
                Err(_failure) => Ok(json!({"missing_key": true})),
            }
        }
        Call::Request {
            prompt,
            optional_params,
        } => {
            // ast-grep-ignore: no-long-names
            let request = StabilityImageGenerationConfig::transform_image_generation_request(
                prompt,
                optional_params,
            );
            match request {
                Ok(request) => Ok(json!({"request": request})),
                Err(_failure) => Ok(json!({"invalid": true})),
            }
        }
        Call::Response {
            status,
            headers,
            body,
            created,
            data,
        } => {
            let upstream = ImageHttpResponse {
                status_code: *status,
                headers: headers.clone(),
                body: body.clone().into_bytes(),
            };
            let mut response = ImageResponse {
                created: *created,
                data: (!data.is_empty()).then(|| data.clone()),
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
    }
}
