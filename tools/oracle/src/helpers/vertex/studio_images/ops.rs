use std::collections::BTreeMap;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::image_contracts::records::ImageInputTokens;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::studio_image_generation::cost::{ImageCostResponse, generation_cost};
use betterlite_vertex::studio_image_generation::environment::validate_environment;
use betterlite_vertex::studio_image_generation::error::StudioImageError;
use betterlite_vertex::studio_image_generation::factory::GOOGLE_IMAGE_GEN_CONFIG;
use betterlite_vertex::studio_image_generation::options::map_openai_params;
// ast-grep-ignore: no-long-names
use betterlite_vertex::studio_image_generation::request::transform_image_generation_request;
use betterlite_vertex::studio_image_generation::response::{
    ImageGenerationResponse,
    // ast-grep-ignore: no-long-names
    transform_image_generation_response,
};
use betterlite_vertex::studio_image_generation::settings::{DEFAULT_API_BASE, Settings};
use betterlite_vertex::studio_image_generation::url::complete_url;
use betterlite_vertex::studio_image_generation::usage::ImageGenerationUsage;
use betterlite_vertex::vertex_gemini_images::response::GeneratedImage;
use http::header::HeaderMap;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use super::pricing::CatalogPricing;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) factories: Vec<String>,
    pub(super) requests: Vec<Request>,
    pub(super) responses: Vec<ResponseCase>,
    pub(super) costs: Vec<CostCase>,
}

// one validate_environment and request transform: the key sources, the base and the caller's headers
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    model: String,
    prompt: String,
    #[serde(default)]
    params: Object,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    image_size_support: Option<bool>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResponseCase {
    model: String,
    body: Value,
}

// one cost call: the catalog entry the lookup answers with and the ImageResponse it prices
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostCase {
    model: String,
    catalog: PriceRecord,
    images: usize,
    #[serde(default)]
    usage: Option<CostUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostUsage {
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    #[serde(default)]
    text_tokens: i64,
    #[serde(default)]
    image_tokens: i64,
    #[serde(default)]
    web_search_requests: Option<i64>,
}

pub(super) fn run_cases(input: &Input) -> HelperResult {
    let factories: Vec<Value> = input
        .factories
        .iter()
        .map(|model| json!({"model": model, "config": GOOGLE_IMAGE_GEN_CONFIG}))
        .collect();
    let requests = input
        .requests
        .iter()
        .map(request_value)
        .collect::<Result<Vec<_>, _>>()?;
    let responses = input
        .responses
        .iter()
        .map(response_value)
        .collect::<Result<Vec<_>, _>>()?;
    let costs = input
        .costs
        .iter()
        .map(|case| Ok(json!({"cost": priced_cost(case)?.normalize().to_string()})))
        .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
    Ok(json!({
        "factories": factories,
        "requests": requests,
        "responses": responses,
        "costs": costs,
    }))
}

fn settings(env: &BTreeMap<String, String>) -> Settings {
    Settings {
        gemini_api_key: env
            .get("GEMINI_API_KEY")
            .filter(|value| !value.is_empty())
            .map(|value| SecretString::from(value.as_str())),
        api_base: env
            .get("GEMINI_API_BASE")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
    }
}

fn request_value(case: &Request) -> Result<Value, Box<HelperError>> {
    let settings = settings(&case.env);
    let key = case.api_key.as_deref().map(SecretString::from);
    let url = complete_url(&case.model, case.api_base.as_deref(), &settings);
    let headers = validate_environment(&case.headers, key.as_ref(), &settings).or_value_error()?;
    let mapped = map_openai_params(
        &case.params,
        &Object::new(),
        &case.model,
        case.image_size_support,
    )
    .or_value_error()?;
    // ast-grep-ignore: no-long-names
    let body = transform_image_generation_request(
        &case.model,
        &case.prompt,
        &mapped,
        case.image_size_support,
    )
    .or_value_error()?;
    let revealed: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect();
    Ok(json!({"url": url, "headers": revealed, "body": body}))
}

fn response_value(case: &ResponseCase) -> Result<Value, Box<HelperError>> {
    let body = serde_json::to_vec(&case.body).or_config_error()?;
    // ast-grep-ignore: no-long-names
    let response = transform_image_generation_response(
        &case.model,
        &UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body,
        },
    )
    .or_value_error()?;
    let usage = match response.usage.as_ref() {
        Some(usage) => usage_value(usage)?,
        None => Value::Null,
    };
    Ok(json!({
        "images": response.data.iter().map(image_value).collect::<Vec<Value>>(),
        "usage": usage,
    }))
}

fn image_value(image: &GeneratedImage) -> Value {
    json!({
        "b64_json": image.b64_json.as_deref(),
        "thought_signature": image
            .provider_specific_fields
            .as_ref()
            .map(|fields| fields.thought_signature.as_str()),
    })
}

// The encoded usage skips an absent search count, while the helper's Python dict keeps the key as null.
fn usage_value(usage: &ImageGenerationUsage) -> Result<Value, Box<HelperError>> {
    let mut encoded = serde_json::to_value(usage).or_config_error()?;
    if let Value::Object(fields) = &mut encoded {
        fields.insert(
            "web_search_requests".to_owned(),
            json!(usage.web_search_requests),
        );
    }
    Ok(encoded)
}

fn priced_cost(case: &CostCase) -> Result<Decimal, Box<HelperError>> {
    let response = ImageGenerationResponse {
        data: image_records(case.images),
        usage: case.usage.as_ref().map(cost_usage),
    };
    let pricing = CatalogPricing {
        catalog: &case.catalog,
    };
    generation_cost(
        &pricing,
        &case.model,
        None,
        ImageCostResponse::Image(&response),
    )
    .map_err(|error| cost_failure(&error))
}

fn image_records(count: usize) -> Vec<GeneratedImage> {
    let image = GeneratedImage {
        b64_json: Some("AAA".to_owned()),
        url: None,
        revised_prompt: None,
        provider_specific_fields: None,
    };
    vec![image; count]
}

// the usage the response would carry: the itemized input details and the images taking the whole output
fn cost_usage(usage: &CostUsage) -> ImageGenerationUsage {
    let input_details = ImageInputTokens {
        image_tokens: usage.image_tokens,
        text_tokens: usage.text_tokens,
    };
    let output_details = ImageInputTokens {
        image_tokens: usage.output_tokens,
        text_tokens: 0,
    };
    ImageGenerationUsage {
        input_tokens: usage.input_tokens,
        input_tokens_details: input_details.clone(),
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens: usage.input_tokens,
        prompt_tokens_details: input_details,
        completion_tokens: usage.output_tokens,
        completion_tokens_details: output_details.clone(),
        output_tokens_details: output_details,
        web_search_requests: usage.web_search_requests,
    }
}

fn cost_failure(error: &StudioImageError) -> Box<HelperError> {
    let class = match error {
        StudioImageError::Cost(ImageCostError::NotImageResponse) => "ValueError",
        StudioImageError::Cost(_) => "ImageCostError",
        StudioImageError::Encode(_)
        | StudioImageError::FieldType { .. }
        | StudioImageError::MissingApiKey
        | StudioImageError::NotJson { .. }
        | StudioImageError::Policy(_)
        | StudioImageError::Shape { .. }
        | StudioImageError::TokenOverflow { .. }
        | StudioImageError::TooLarge { .. }
        | StudioImageError::UsageShape => "ValueError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: error.upstream_answer().map(|(status, _headers)| status),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
