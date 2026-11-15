use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_core::api::recraft::cost::cost_calculator;
use betterlite_core::api::recraft::edit::RecraftImageEditConfig;
use betterlite_core::api::recraft::generation::RecraftImageGenerationConfig;
// ast-grep-ignore: no-long-names
use betterlite_core::api::recraft::generation::get_recraft_image_generation_config as generation_config;
use betterlite_core::api::recraft::reply::RecraftReplyError;
use betterlite_core::api::recraft::settings::RecraftImageSettings;
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageFile, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
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

// No case replaces the shared cost map, so every price comes from the record the call carries.
struct NoPrices;

impl PriceLookup for NoPrices {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
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

fn supported_answer(endpoint: Endpoint, _model: &str) -> Value {
    let supported: &[&str] = match endpoint {
        Endpoint::Generation => &RecraftImageGenerationConfig::SUPPORTED_OPENAI_PARAMS,
        Endpoint::Edit => &RecraftImageEditConfig::SUPPORTED_OPENAI_PARAMS,
    };
    Value::Array(
        supported
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
    let attempt = RecraftImageGenerationConfig::map_openai_params(
        non_default_params,
        optional_params.clone(),
        model,
        drop_params,
    );
    match attempt {
        Ok(mapped) => json!({"mapped": mapped}),
        Err(_unsupported) => json!({"unsupported": true}),
    }
}

fn edit_map_answer(model: &str, params: &ImageParams, drop_params: bool) -> Value {
    json!({
        "mapped": RecraftImageEditConfig::map_openai_params(params.clone(), model, drop_params)
    })
}

fn url_answer(
    endpoint: Endpoint,
    model: &str,
    api_base: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Value {
    let url = match endpoint {
        Endpoint::Generation => generation_config(model, environment).get_complete_url(api_base),
        Endpoint::Edit => RecraftImageEditConfig {
            settings: RecraftImageSettings::from_environment(environment),
        }
        .get_complete_url(api_base),
    };
    json!({"url": url})
}

fn environment_answer(
    endpoint: Endpoint,
    model: &str,
    api_key: Option<&str>,
    headers: &ImageHeaders,
    environment: &BTreeMap<String, String>,
) -> Value {
    let explicit = api_key.map(SecretString::from);
    let validated = match endpoint {
        Endpoint::Generation => generation_config(model, environment)
            .validate_environment(headers.clone(), explicit.as_ref()),
        Endpoint::Edit => RecraftImageEditConfig {
            settings: RecraftImageSettings::from_environment(environment),
        }
        .validate_environment(headers.clone(), explicit.as_ref()),
    };
    match validated {
        Ok(headers) => json!({"headers": headers}),
        Err(_missing_key) => json!({"missing_key": true}),
    }
}

fn request_answer(
    model: &str,
    prompt: &str,
    optional_params: &ImageParams,
) -> Result<Value, Box<HelperError>> {
    // ast-grep-ignore: no-long-names
    let attempt = RecraftImageGenerationConfig::transform_image_generation_request(
        model,
        prompt,
        optional_params,
    );
    match attempt {
        Ok(request) => Ok(json!({"request": request})),
        Err(error) => Err(helper_error("TypeError", error.to_string())),
    }
}

fn edit_request_answer(
    model: &str,
    prompt: Option<&str>,
    images: &[ImageSeed],
    optional_params: &ImageParams,
) -> Result<Value, Box<HelperError>> {
    let files = images
        .iter()
        .map(seed_file)
        .collect::<Result<Vec<ImageFile>, Box<HelperError>>>()?;
    let (data, parts) = RecraftImageEditConfig::transform_image_edit_request(
        model,
        prompt,
        Some(files.as_slice()),
        optional_params.clone(),
    );
    let parts = parts.iter().map(part_value).collect::<Vec<Value>>();
    Ok(json!({"data": data, "files": parts}))
}

fn seed_file(seed: &ImageSeed) -> Result<ImageFile, Box<HelperError>> {
    let content = BASE64
        .decode(&seed.data_base64)
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(ImageFile {
        filename: seed.filename.clone(),
        content,
        content_type: None,
    })
}

fn part_value((field, file): &(String, ImageFile)) -> Value {
    json!({
        "field": field,
        "filename": file.filename,
        "content_type": file.content_type,
        "content": BASE64.encode(&file.content),
    })
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
    // ast-grep-ignore: no-long-names
    match RecraftImageGenerationConfig::transform_image_generation_response(
        &upstream,
        &mut response,
    ) {
        Ok(()) => Ok(json!({"response": response})),
        Err(error) => Err(reply_error(&error)),
    }
}

fn edit_response_answer(
    status: u16,
    headers: &ImageHeaders,
    body: &str,
) -> Result<Value, Box<HelperError>> {
    let upstream = ImageHttpResponse {
        status_code: status,
        headers: headers.clone(),
        body: body.as_bytes().to_vec(),
    };
    match RecraftImageEditConfig::transform_image_edit_response(&upstream, 0) {
        Ok(response) => Ok(json!({"data": response.data})),
        Err(error) => Err(reply_error(&error)),
    }
}

fn reply_error(error: &RecraftReplyError) -> Box<HelperError> {
    let mut failure = helper_error("BaseLLMException", error.to_string());
    failure.status = error.status();
    failure
}

fn cost_answer(
    model: &str,
    output_cost_per_image: Option<&Scalar>,
    image_count: i64,
    other: bool,
) -> Result<Value, Box<HelperError>> {
    let mut model_info = PriceRecord::new();
    if let Some(rate) = output_cost_per_image {
        model_info.insert("output_cost_per_image".to_owned(), rate.clone());
    }
    let billing = ImageBilling {
        image_count,
        size: None,
        quality: None,
        usage: None,
    };
    let response = if other {
        ImageResponseView::Other
    } else {
        ImageResponseView::Image(&billing)
    };
    let cost = cost_calculator(model, response, Some(&model_info), &NoPrices)
        .map_err(|error| helper_error("ImageCostError", error.to_string()))?;
    Ok(json!({"cost": cost.normalize().to_string()}))
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
