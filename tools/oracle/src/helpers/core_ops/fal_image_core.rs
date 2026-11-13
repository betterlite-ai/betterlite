use std::collections::BTreeMap;

use betterlite_config::sections::Sections;
use betterlite_core::api::fal::gpt_image::complete_url as gpt_complete_url;
use betterlite_core::api::fal::gpt_image::params::map_openai_params as map_gpt_params;
use betterlite_core::api::fal::gpt_image::quality::{
    map_gpt_image_quality, supported_gpt_image_qualities,
};
use betterlite_core::api::fal::gpt_image::size::map_gpt_image_size;
use betterlite_core::api::fal::images::decode::{
    FalImage, FalImageProviderSpecificFields, append_generation_images,
};
use betterlite_core::api::fal::images::params::map_openai_params;
use betterlite_core::api::fal::images::settings::FalImageSettings;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

type Environment = BTreeMap<String, String>;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    GenericBaseOnly {
        environment: Environment,
        api_bases: Vec<Option<String>>,
        models: Vec<String>,
        model: String,
        cases: Vec<MappingCase>,
    },
    KeyScheme {
        environment: Environment,
        cases: Vec<KeyCase>,
    },
    MixedImageObjects {
        replies: Vec<ReplyCase>,
    },
    BoolDimensionsExcluded {
        replies: Vec<ReplyCase>,
    },
    GptDestinationPrecedence {
        model: String,
        cases: Vec<MappingCase>,
    },
    PricingQualityAlias {
        model: String,
        catalog: IndexMap<String, Json>,
        qualities: Vec<Json>,
    },
    MalformedSizeKept {
        sizes: Vec<Json>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingCase {
    #[serde(default)]
    non_default: ImageParams,
    #[serde(default)]
    optional: ImageParams,
    #[serde(default)]
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyCase {
    #[serde(default)]
    headers: ImageHeaders,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    status: u16,
    #[serde(default)]
    headers: ImageHeaders,
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::GenericBaseOnly {
                environment,
                api_bases,
                models,
                model,
                cases,
            } => {
                let settings = settings(environment);
                let mut urls = Vec::new();
                for api_base in api_bases {
                    for candidate in models {
                        urls.push(json!({
                            "generic": settings.complete_url(api_base.as_deref(), ""),
                            "gpt": gpt_complete_url(&settings, api_base.as_deref(), candidate),
                        }));
                    }
                }
                let outcomes: Vec<Value> = cases
                    .iter()
                    .map(|case| generic_outcome(case, model))
                    .collect();
                Ok(json!({"urls": urls, "cases": outcomes}))
            }
            Input::KeyScheme { environment, cases } => {
                let settings = settings(environment);
                let outcomes: Vec<Value> = cases
                    .iter()
                    .map(|case| key_outcome(case, &settings))
                    .collect();
                Ok(json!({"cases": outcomes}))
            }
            Input::MixedImageObjects { replies } | Input::BoolDimensionsExcluded { replies } => {
                let outcomes: Vec<Value> = replies.iter().map(reply_outcome).collect();
                Ok(json!({"cases": outcomes}))
            }
            Input::GptDestinationPrecedence { model, cases } => {
                let catalog = BundledCatalog::default().get();
                let outcomes: Vec<Value> = cases
                    .iter()
                    .map(|case| gpt_outcome(case, model, &catalog))
                    .collect();
                Ok(json!({"cases": outcomes}))
            }
            Input::PricingQualityAlias {
                model,
                catalog,
                qualities,
            } => {
                let catalog = catalog_from(catalog)?;
                Ok(quality_outcome(model, &catalog, qualities))
            }
            Input::MalformedSizeKept { sizes } => {
                let mapped: Vec<Value> = sizes
                    .iter()
                    .map(|size| json!(map_gpt_image_size(size)))
                    .collect();
                Ok(json!({"sizes": mapped}))
            }
        }
    }
}

fn settings(environment: &Environment) -> FalImageSettings {
    let (api_key, api_base) = Sections::fal_image_environment(environment);
    FalImageSettings { api_key, api_base }
}

fn generic_outcome(case: &MappingCase, model: &str) -> Value {
    match map_openai_params(
        &case.non_default,
        case.optional.clone(),
        model,
        case.drop_params,
    ) {
        Ok(mapped) => json!({"params": mapped}),
        Err(_) => json!({"error": {"status": null}}),
    }
}

fn gpt_outcome(case: &MappingCase, model: &str, catalog: &ModelCatalog) -> Value {
    match map_gpt_params(
        &case.non_default,
        case.optional.clone(),
        model,
        case.drop_params,
        catalog,
    ) {
        Ok(mapped) => json!({"params": mapped}),
        Err(_) => json!({"error": {"status": null}}),
    }
}

fn key_outcome(case: &KeyCase, settings: &FalImageSettings) -> Value {
    let explicit = case.api_key.as_deref().map(SecretString::from);
    match settings.authorize(&case.headers, explicit.as_ref()) {
        Ok(headers) => json!({"headers": headers}),
        Err(_) => json!({"error": {"status": null}}),
    }
}

fn reply_outcome(case: &ReplyCase) -> Value {
    let mut data = None;
    match append_generation_images(&mut data, case.status, &case.headers, case.body.as_bytes()) {
        Ok(()) => json!({"images": records(data.as_deref().unwrap_or_default())}),
        Err(_) => json!({"error": {"status": case.status}}),
    }
}

fn records(images: &[FalImage]) -> Vec<Value> {
    images.iter().map(record).collect()
}

fn record(image: &FalImage) -> Value {
    json!({
        "url": image.url.as_deref(),
        "b64_json": image.b64_json.as_deref(),
        "provider_specific_fields": image.provider_specific_fields.as_ref().map(provider_fields),
    })
}

fn provider_fields(fields: &FalImageProviderSpecificFields) -> Value {
    let mut object = BTreeMap::new();
    if let Some(width) = fields.width {
        object.insert("width".to_owned(), json!(width));
    }
    if let Some(height) = fields.height {
        object.insert("height".to_owned(), json!(height));
    }
    if let Some(content_type) = fields.content_type.as_deref() {
        object.insert("content_type".to_owned(), json!(content_type));
    }
    json!(object)
}

fn catalog_from(entries: &IndexMap<String, Json>) -> Result<ModelCatalog, Box<HelperError>> {
    let bytes = serde_json::to_vec(entries).map_err(|error| helper_error(error.to_string()))?;
    ModelCatalog::parse(&bytes, UnknownFields::Reject)
        .map_err(|error| helper_error(error.to_string()))
}

fn quality_outcome(model: &str, catalog: &ModelCatalog, qualities: &[Json]) -> Value {
    let supported: Vec<String> = supported_gpt_image_qualities(model, catalog)
        .into_iter()
        .collect();
    let mapped: Vec<Value> = qualities
        .iter()
        .map(|quality| json!(map_gpt_image_quality(quality, model, catalog)))
        .collect();
    json!({"supported": supported, "mapped": mapped})
}

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
