use std::collections::BTreeMap;
use std::io::Cursor;

use betterlite_core::api::image_input_options::mime::image_content_type_stream;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::studio_image_edit::factory::GEMINI_IMAGE_EDIT_CONFIG;
use betterlite_vertex::studio_image_edit::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_vertex::studio_image_edit::request::{
    MULTIPART_FORM_DATA, transform_image_edit_request,
};
use betterlite_vertex::studio_image_edit::url::complete_url;
use betterlite_vertex::studio_image_generation::environment::validate_environment;
use betterlite_vertex::studio_image_generation::settings::{DEFAULT_API_BASE, Settings};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use super::costs::cost_value;
use super::errors::edit_failure;
use super::images::{ImageField, edit_images, positions_value, prepared_field};
use super::responses::response_value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) factories: Vec<String>,
    pub(super) requests: Vec<Request>,
    pub(super) responses: Vec<ResponseCase>,
    pub(super) costs: Vec<CostCase>,
}

// one mapped edit request: the key sources, the base, the caller's headers and the image records
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    model: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    image: Option<ImageField>,
    #[serde(default)]
    params: Object,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResponseCase {
    pub(super) body: Value,
}

// one cost call: the catalog entry the lookup answers with and the ImageResponse it prices
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostCase {
    pub(super) model: String,
    pub(super) catalog: PriceRecord,
    pub(super) images: usize,
    #[serde(default)]
    pub(super) usage: Option<CostUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostUsage {
    pub(super) input_tokens: i64,
    pub(super) output_tokens: i64,
    pub(super) total_tokens: i64,
    #[serde(default)]
    pub(super) text_tokens: i64,
    #[serde(default)]
    pub(super) image_tokens: i64,
}

pub(super) fn run_cases(input: &Input) -> HelperResult {
    let factories: Vec<Value> = input
        .factories
        .iter()
        .map(|model| json!({"model": model, "config": GEMINI_IMAGE_EDIT_CONFIG}))
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
        .map(cost_value)
        .collect::<Result<Vec<_>, _>>()?;
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
    let mapped =
        map_openai_params(&case.params, &case.model).map_err(|error| edit_failure(&error))?;
    let url = complete_url(&case.model, case.api_base.as_deref(), &settings);
    let key = case.api_key.as_deref().map(SecretString::from);
    let headers = validate_environment(&case.headers, key.as_ref(), &settings).or_value_error()?;
    let mut prepared = prepared_field(case.image.as_ref())?;
    let mut view = edit_images(&mut prepared);
    // B033 injects the same core sniffer, so the oracle compares its answer with LiteLLM's reader
    let sniff = |content: &[u8]| image_content_type_stream(&mut Cursor::new(content));
    let (payload, files) = transform_image_edit_request(
        &case.model,
        case.prompt.as_deref(),
        view.as_mut(),
        &mapped,
        &sniff,
    )
    .map_err(|error| edit_failure(&error))?;
    let revealed: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect();
    Ok(json!({
        "url": url,
        "headers": revealed,
        "supported": SUPPORTED_OPENAI_PARAMS,
        "multipart": MULTIPART_FORM_DATA,
        "mapped": mapped,
        "payload": payload,
        "files": files.len(),
        "positions": positions_value(&prepared),
    }))
}
