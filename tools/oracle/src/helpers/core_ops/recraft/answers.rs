use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::types::RoutingStrategy;
use betterlite_core::api::recraft::cost::cost_calculator;
use betterlite_core::api::recraft::edit::RecraftImageEditConfig;
use betterlite_core::api::recraft::error::RecraftError;
use betterlite_core::api::recraft::generation::RecraftImageGenerationConfig;
// ast-grep-ignore: no-long-names
use betterlite_core::api::recraft::generation::get_recraft_image_generation_config as generation_config;
use betterlite_core::api::recraft::reply::RecraftReplyError;
use betterlite_core::api::recraft::settings::RecraftImageSettings;
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageHeaders, ImageHttpResponse, ImageResponse,
};
use betterlite_oracle_helper_api::error::HelperError;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{
    CostCall, EditMapCall, EditRequestCall, Endpoint, EnvironmentCall, ImageSeed, MapCall,
    ReplyCall, RequestCall, UrlCall,
};

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

// LiteLLM resolves the url and the headers through one class per endpoint, so the answers pick a config the same way
enum Configured {
    Generation(RecraftImageGenerationConfig),
    Edit(RecraftImageEditConfig),
}

impl Configured {
    fn complete_url(&self, api_base: Option<&str>) -> String {
        match self {
            Self::Generation(config) => config.get_complete_url(api_base),
            Self::Edit(config) => config.get_complete_url(api_base),
        }
    }

    fn validate_environment(
        &self,
        headers: ImageHeaders,
        api_key: Option<&SecretString>,
    ) -> Result<ImageHeaders, RecraftError> {
        match self {
            Self::Generation(config) => config.validate_environment(headers, api_key),
            Self::Edit(config) => config.validate_environment(headers, api_key),
        }
    }
}

fn endpoint_config(
    endpoint: Endpoint,
    model: &str,
    environment: &BTreeMap<String, String>,
) -> Configured {
    match endpoint {
        Endpoint::Generation => Configured::Generation(generation_config(model, environment)),
        Endpoint::Edit => Configured::Edit(edit_config(environment)),
    }
}

fn edit_config(environment: &BTreeMap<String, String>) -> RecraftImageEditConfig {
    let (api_key, api_base) = RoutingStrategy::recraft_image_settings(environment);
    RecraftImageEditConfig {
        settings: RecraftImageSettings { api_key, api_base },
    }
}

pub(super) fn supported_answer(endpoint: Endpoint, _model: &str) -> Value {
    let supported: &[&str] = match endpoint {
        Endpoint::Generation => &RecraftImageGenerationConfig::SUPPORTED_OPENAI_PARAMS,
        Endpoint::Edit => &RecraftImageEditConfig::SUPPORTED_OPENAI_PARAMS,
    };
    json!(supported)
}

pub(super) fn map_answer(call: &MapCall) -> Value {
    let attempt = RecraftImageGenerationConfig::map_openai_params(
        &call.non_default_params,
        call.optional_params.clone(),
        &call.model,
        call.drop_params,
    );
    match attempt {
        Ok(mapped) => json!({"mapped": mapped}),
        Err(_unsupported) => json!({"unsupported": true}),
    }
}

pub(super) fn edit_map_answer(call: &EditMapCall) -> Value {
    json!({
        "mapped": RecraftImageEditConfig::map_openai_params(
            call.params.clone(),
            &call.model,
            call.drop_params,
        )
    })
}

pub(super) fn url_answer(call: &UrlCall) -> Value {
    json!({
        "url": endpoint_config(call.endpoint, &call.model, &call.environment)
            .complete_url(call.api_base.as_deref())
    })
}

pub(super) fn environment_answer(call: &EnvironmentCall) -> Value {
    let explicit = call.api_key.as_deref().map(SecretString::from);
    endpoint_config(call.endpoint, &call.model, &call.environment)
        .validate_environment(call.headers.clone(), explicit.as_ref())
        .map_or_else(
            |_missing_key| json!({"missing_key": true}),
            |headers| json!({"headers": headers}),
        )
}

pub(super) fn request_answer(call: &RequestCall) -> Result<Value, Box<HelperError>> {
    // ast-grep-ignore: no-long-names
    RecraftImageGenerationConfig::transform_image_generation_request(
        &call.model,
        &call.prompt,
        &call.optional_params,
    )
    .map(|request| json!({"request": request}))
    .map_err(|error| helper_error("TypeError", error.to_string()))
}

pub(super) fn edit_request_answer(call: &EditRequestCall) -> Result<Value, Box<HelperError>> {
    let files = call
        .images
        .iter()
        .map(seed_file)
        .collect::<Result<Vec<ImageFile>, Box<HelperError>>>()?;
    let (data, uploaded) = RecraftImageEditConfig::transform_image_edit_request(
        &call.model,
        call.prompt.as_deref(),
        Some(files.as_slice()),
        call.optional_params.clone(),
    );
    let parts = uploaded.iter().map(part_value).collect::<Vec<Value>>();
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

pub(super) fn response_answer(
    endpoint: Endpoint,
    call: &ReplyCall,
) -> Result<Value, Box<HelperError>> {
    let upstream = upstream_response(call);
    match endpoint {
        Endpoint::Generation => {
            let mut response = ImageResponse {
                created: call.created,
                data: (!call.data.is_empty()).then(|| call.data.clone()),
                usage: None,
            };
            // ast-grep-ignore: no-long-names
            RecraftImageGenerationConfig::transform_image_generation_response(
                &upstream,
                &mut response,
            )
            .map_err(|error| reply_error(&error))?;
            Ok(json!({"response": response}))
        }
        Endpoint::Edit => {
            RecraftImageEditConfig::transform_image_edit_response(&upstream, call.created)
                .map(|edited| json!({"data": edited.data, "usage": edited.usage}))
                .map_err(|error| reply_error(&error))
        }
    }
}

fn upstream_response(call: &ReplyCall) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: call.status,
        headers: call.headers.clone(),
        body: call.body.as_bytes().to_vec(),
    }
}

fn reply_error(error: &RecraftReplyError) -> Box<HelperError> {
    let mut failure = helper_error("BaseLLMException", error.to_string());
    failure.status = error.malformed().map(|(status, _headers)| status);
    failure
}

pub(super) fn cost_answer(call: &CostCall) -> Result<Value, Box<HelperError>> {
    let mut model_info = PriceRecord::new();
    if let Some(rate) = call.output_cost_per_image.as_ref() {
        model_info.insert("output_cost_per_image".to_owned(), rate.clone());
    }
    let billing = ImageBilling {
        image_count: call.image_count,
        size: None,
        quality: None,
        usage: None,
    };
    let response = if call.other {
        ImageResponseView::Other
    } else {
        ImageResponseView::Image(&billing)
    };
    let cost = cost_calculator(&call.model, response, Some(&model_info), &NoPrices)
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
