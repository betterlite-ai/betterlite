use betterlite_openai::watsonx_auth::credentials::get_api_params;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::payload::prepare_payload;
use betterlite_openai::watsonx_auth::types::{WatsonxEndpoint, WatsonxParams};
use betterlite_openai::watsonx_auth::urls::{add_api_version_to_url, get_base_url};
use serde::Serialize;

use super::error::WatsonxTextError;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 7] = [
    "temperature",
    "max_tokens",
    "top_p",
    "frequency_penalty",
    "stop",
    "seed",
    "stream",
];

pub const SPECIAL_AUTH_PARAMS: [(&str, &str); 3] = [
    ("project", "watsonx_project"),
    ("region_name", "watsonx_region_name"),
    ("token", "watsonx_token"),
];

pub const EU_REGIONS: [&str; 2] = ["eu-de", "eu-gb"];
pub const US_REGIONS: [&str; 1] = ["us-south"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextGenerationBody {
    pub input: Option<String>,
    pub moderations: WatsonxParams,
    pub parameters: WatsonxParams,
    #[serde(flatten)]
    pub identity: WatsonxParams,
}

#[must_use]
pub fn is_watsonx_text_param(param: &str) -> bool {
    [
        "decoding_method",
        "max_new_tokens",
        "min_new_tokens",
        "length_penalty",
        "stop_sequences",
        "top_k",
        "repetition_penalty",
        "truncate_input_tokens",
        "include_stop_sequences",
        "return_options",
        "random_seed",
        "moderations",
        "min_tokens",
    ]
    .contains(&param)
}

enum Mapped {
    Top(&'static str),
    Extra(&'static str),
}

fn mapped_name(key: &str) -> Option<Mapped> {
    match key {
        "max_tokens" => Some(Mapped::Top("max_new_tokens")),
        "stream" => Some(Mapped::Top("stream")),
        "temperature" => Some(Mapped::Top("temperature")),
        "top_p" => Some(Mapped::Top("top_p")),
        "frequency_penalty" => Some(Mapped::Top("repetition_penalty")),
        "seed" => Some(Mapped::Top("random_seed")),
        "stop" => Some(Mapped::Top("stop_sequences")),
        "decoding_method" => Some(Mapped::Extra("decoding_method")),
        "min_tokens" => Some(Mapped::Extra("min_new_tokens")),
        "top_k" => Some(Mapped::Extra("top_k")),
        "truncate_input_tokens" => Some(Mapped::Extra("truncate_input_tokens")),
        "length_penalty" => Some(Mapped::Extra("length_penalty")),
        "time_limit" => Some(Mapped::Extra("time_limit")),
        "return_options" => Some(Mapped::Extra("return_options")),
        _ => None,
    }
}

pub fn map_openai_params(non_default: &WatsonxParams, options: &mut WatsonxParams) {
    let mut extra_body = WatsonxParams::new();
    for (key, value) in non_default {
        match mapped_name(key) {
            Some(Mapped::Top(name)) => {
                options.insert(name.to_owned(), value.clone());
            }
            Some(Mapped::Extra(name)) => {
                extra_body.insert(name.to_owned(), value.clone());
            }
            None => {}
        }
    }
    if !extra_body.is_empty() {
        options.insert(String::from("extra_body"), serde_json::json!(extra_body));
    }
}

pub fn map_special_auth_params(non_default: &WatsonxParams, options: &mut WatsonxParams) {
    for (param, mapped) in SPECIAL_AUTH_PARAMS {
        if let Some(value) = non_default.get(param) {
            options.insert(mapped.to_owned(), value.clone());
        }
    }
}

pub fn build_body(
    model: &str,
    prompt: Option<&str>,
    options: WatsonxParams,
    environment: &WatsonxEnvironment,
) -> Result<TextGenerationBody, WatsonxTextError> {
    let mut parameters = options;
    let extra = pop_mapping(&mut parameters, "extra_body")?;
    parameters.extend(extra);
    let api_params = get_api_params(&mut parameters, Some(model), environment)?;
    let identity = prepare_payload(model, &api_params);
    let moderations = pop_mapping(&mut parameters, "moderations")?;
    Ok(TextGenerationBody {
        input: prompt.map(str::to_owned),
        moderations,
        parameters,
        identity,
    })
}

pub fn complete_url(
    environment: &WatsonxEnvironment,
    api_base: Option<&str>,
    model: &str,
    options: &mut WatsonxParams,
    stream: bool,
) -> Result<String, WatsonxTextError> {
    let base = get_base_url(api_base, environment)?;
    let endpoint = match (model.strip_prefix("deployment/"), stream) {
        (Some(deployment), true) => WatsonxEndpoint::DeploymentTextGenerationStream
            .path()
            .replace("{deployment_id}", deployment),
        (Some(deployment), false) => WatsonxEndpoint::DeploymentTextGeneration
            .path()
            .replace("{deployment_id}", deployment),
        (None, true) => String::from(WatsonxEndpoint::TextGenerationStream.path()),
        (None, false) => String::from(WatsonxEndpoint::TextGeneration.path()),
    };
    let url = format!("{}{endpoint}", base.trim_end_matches('/'));
    let version = match options.remove("api_version") {
        None => None,
        Some(value) => serde_json::from_value::<Option<String>>(value).map_err(|source| {
            WatsonxTextError::InvalidField {
                field: "api_version",
                source,
            }
        })?,
    };
    Ok(add_api_version_to_url(&url, version.as_deref()))
}

fn pop_mapping(
    params: &mut WatsonxParams,
    field: &'static str,
) -> Result<WatsonxParams, WatsonxTextError> {
    let Some(value) = params.remove(field) else {
        return Ok(WatsonxParams::new());
    };
    serde_json::from_value::<Option<WatsonxParams>>(value)
        .map(Option::unwrap_or_default)
        .map_err(|source| WatsonxTextError::NotAMapping { field, source })
}
