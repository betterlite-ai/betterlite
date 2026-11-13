use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::reply::ImageDatum;
use http::HeaderMap;
use http::header::{ACCEPT, CONTENT_TYPE, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::Number;

use crate::api::bfl::common::{
    BflError, BflImageEntry, BflReply, BflResult, DEFAULT_API_BASE, IMAGE_GENERATION_MODELS,
};

mod tests;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 11] = [
    "n",
    "size",
    "quality",
    "seed",
    "output_format",
    "safety_tolerance",
    "prompt_upsampling",
    "raw",
    "num_images",
    "image_url",
    "image_prompt_strength",
];

const REQUEST_PARAMS: [&str; 11] = [
    "width",
    "height",
    "aspect_ratio",
    "seed",
    "output_format",
    "safety_tolerance",
    "prompt_upsampling",
    "raw",
    "num_images",
    "image_url",
    "image_prompt_strength",
];

const SIZE_MAPPING: [(&str, i64, i64); 5] = [
    ("1024x1024", 1024, 1024),
    ("1792x1024", 1792, 1024),
    ("1024x1792", 1024, 1792),
    ("512x512", 512, 512),
    ("256x256", 256, 256),
];

const DEFAULT_OUTPUT_FORMAT: &str = "png";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationConfig;

impl GenerationConfig {
    pub fn for_model(model: &str) -> Result<Self, BflError> {
        generation_endpoint(model)?;
        Ok(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BflImageResponse {
    pub created: i64,
    pub data: Vec<ImageDatum>,
}

pub fn generation_endpoint(model: &str) -> Result<&'static str, BflError> {
    let lowered = model.to_lowercase();
    let name = lowered.rsplit('/').next().unwrap_or_default();
    IMAGE_GENERATION_MODELS
        .iter()
        .find_map(|(candidate, endpoint)| (*candidate == name).then_some(*endpoint))
        .ok_or_else(|| BflError::UnknownModel {
            model: name.to_owned(),
        })
}

pub fn complete_url(
    api_base: Option<&str>,
    model: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<String, BflError> {
    let endpoint = generation_endpoint(model)?;
    let base = api_base
        .filter(|base| !base.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            lookup
                .lookup("BFL_API_BASE")
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    Ok(format!("{}{endpoint}", base.trim_end_matches('/')))
}

pub fn validate_environment(
    mut headers: HeaderMap,
    api_key: Option<&SecretString>,
    lookup: &dyn EnvironmentLookup,
) -> Result<HeaderMap, BflError> {
    let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
    let environment = present("BFL_API_KEY").or_else(|| present("BLACK_FOREST_LABS_API_KEY"));
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| environment.map(SecretString::from))
        .ok_or(BflError::MissingCredentials)?;
    let mut value = HeaderValue::try_from(key.expose_secret().as_str())
        .map_err(|source| BflError::InvalidApiKey { source })?;
    value.set_sensitive(true);
    headers.insert(HeaderName::from_static("x-key"), value);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    Ok(headers)
}

pub fn map_openai_params(
    non_default_params: &IndexMap<String, Json>,
    mut optional_params: IndexMap<String, Json>,
    model: &str,
    drop_params: bool,
) -> Result<IndexMap<String, Json>, BflError> {
    let ultra = model.to_lowercase().contains("ultra");
    for (parameter, value) in non_default_params {
        if optional_params.contains_key(parameter) {
            continue;
        }
        if !SUPPORTED_OPENAI_PARAMS.contains(&parameter.as_str()) {
            if drop_params {
                continue;
            }
            return Err(BflError::UnsupportedParameter {
                parameter: parameter.clone(),
                model: model.to_owned(),
            });
        }
        match parameter.as_str() {
            "size" => map_size(value, &mut optional_params)?,
            "n" => {
                if ultra {
                    optional_params.insert("num_images".to_owned(), value.clone());
                }
            }
            "quality" => {
                if ultra && matches!(value, Json::Text(quality) if quality == "hd") {
                    optional_params.insert("raw".to_owned(), Json::Bool(true));
                }
            }
            _ => {
                optional_params.insert(parameter.clone(), value.clone());
            }
        }
    }
    Ok(optional_params)
}

fn map_size(value: &Json, optional_params: &mut IndexMap<String, Json>) -> Result<(), BflError> {
    let Json::Text(size) = value else {
        return if matches!(value, Json::Null) {
            Ok(())
        } else {
            Err(BflError::InvalidSizeField)
        };
    };
    if size.is_empty() {
        return Ok(());
    }
    if let Some((_, width, height)) = SIZE_MAPPING
        .iter()
        .find(|(candidate, _, _)| *candidate == size.as_str())
    {
        optional_params.insert("width".to_owned(), Json::Number(Number::from(*width)));
        optional_params.insert("height".to_owned(), Json::Number(Number::from(*height)));
        return Ok(());
    }
    let Some((width, height)) = size.split_once('x') else {
        return Ok(());
    };
    let width = width
        .trim()
        .parse::<i64>()
        .ok()
        .ok_or_else(|| BflError::InvalidSize { size: size.clone() })?;
    let height = height
        .trim()
        .parse::<i64>()
        .ok()
        .ok_or_else(|| BflError::InvalidSize { size: size.clone() })?;
    optional_params.insert("width".to_owned(), Json::Number(Number::from(width)));
    optional_params.insert("height".to_owned(), Json::Number(Number::from(height)));
    Ok(())
}

#[must_use]
pub fn request_body(
    prompt: &str,
    optional_params: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut body = IndexMap::new();
    body.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
    for parameter in REQUEST_PARAMS {
        if let Some(value) = optional_params.get(parameter)
            && !matches!(value, Json::Null)
        {
            body.insert(parameter.to_owned(), value.clone());
        }
    }
    if !body.contains_key("output_format") {
        body.insert(
            "output_format".to_owned(),
            Json::Text(DEFAULT_OUTPUT_FORMAT.to_owned()),
        );
    }
    body
}

pub fn apply_image_response(
    reply: &BflReply,
    mut data: Vec<ImageDatum>,
    now_seconds: i64,
) -> Result<BflImageResponse, BflError> {
    let mut urls: Vec<&str> = Vec::new();
    match &reply.result {
        BflResult::Sample(sample) => {
            if let Some(url) = &sample.sample {
                urls.push(url);
            }
        }
        BflResult::List(entries) => {
            for entry in entries {
                match entry {
                    BflImageEntry::Url(url) => urls.push(url),
                    BflImageEntry::Resource(resource) => {
                        if let Some(url) = &resource.url {
                            urls.push(url);
                        }
                    }
                    BflImageEntry::Other(_) => {}
                }
            }
        }
        BflResult::Other(_) => {}
    }
    for url in urls {
        data.push(ImageDatum {
            b64_json: None,
            url: Some(url.to_owned()),
            revised_prompt: None,
            provider_specific_fields: None,
        });
    }
    if data.is_empty() {
        return Err(BflError::NoImageUrl);
    }
    Ok(BflImageResponse {
        created: now_seconds,
        data,
    })
}
