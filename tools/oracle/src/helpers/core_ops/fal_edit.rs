use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::PathBuf;

use betterlite_config::sections::Sections;
use betterlite_core::api::fal::edit::error::FalEditError;
use betterlite_core::api::fal::edit::flux_lora_depth::{self, DEPTH_ENDPOINT};
use betterlite_core::api::fal::edit::image_input::{EditImage, to_data_url};
use betterlite_core::api::fal::edit::params::{GENERIC_OPENAI_PARAMS, map_openai_params};
use betterlite_core::api::fal::edit::reply::transform_image_edit_response;
use betterlite_core::api::fal::edit::selector::{Config, for_model};
use betterlite_core::api::fal::edit::transformation::{
    EditRequest, complete_url, json_image, transform_image_edit_request,
};
use betterlite_core::api::fal::images::settings::FalImageSettings;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use betterlite_openai::image_edits::request::ImageEditForm;
use betterlite_openai::image_generation::reply::ImageDatum;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

type Environment = BTreeMap<String, String>;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ToDataUrl {
        sources: Vec<Source>,
    },
    Prepare {
        model: String,
        prompt: Option<String>,
        image: Option<PrepareImage>,
        #[serde(default)]
        params: ImageParams,
    },
    Map {
        model: String,
        options: Vec<ImageParams>,
    },
    Url {
        environment: Environment,
        api_bases: Vec<Option<String>>,
        models: Vec<String>,
    },
    Reply {
        replies: Vec<ReplyCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Source {
    Bytes { content: Vec<u8> },
    Path { name: String, content: Vec<u8> },
    Reader { content: Vec<u8>, position: u64 },
    Tuple { content: Vec<u8> },
    Value { value: Json },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PrepareImage {
    Bytes { content: Vec<u8> },
    Path { name: String, content: Vec<u8> },
    Reader { content: Vec<u8>, position: u64 },
    Tuple { content: Vec<u8> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    status: u16,
    #[serde(default)]
    headers: ImageHeaders,
    body: String,
}

mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ToDataUrl { sources } => {
                let outcomes = sources
                    .iter()
                    .map(source_outcome)
                    .collect::<HelperResult<Vec<Value>>>()?;
                Ok(json!({ "sources": outcomes }))
            }
            Input::Prepare {
                model,
                prompt,
                image,
                params,
            } => prepare_outcome(model, prompt.as_deref(), image.as_ref(), params),
            Input::Map { model, options } => Ok(map_outcome(model, options)),
            Input::Url {
                environment,
                api_bases,
                models,
            } => Ok(url_outcome(environment, api_bases, models)),
            Input::Reply { replies } => {
                let outcomes: Vec<Value> = replies.iter().map(reply_outcome).collect();
                Ok(json!({ "replies": outcomes }))
            }
        }
    }
}

fn source_outcome(source: &Source) -> HelperResult<Value> {
    match source {
        Source::Bytes { content } => Ok(data_url_result(to_data_url(&EditImage::Bytes(content)))),
        Source::Path { name, content } => {
            let path = fixture_path(name, content)?;
            Ok(data_url_result(to_data_url(&EditImage::Path(&path))))
        }
        Source::Reader { content, position } => {
            let mut reader = Cursor::new(content.clone());
            reader.set_position(*position);
            let result = {
                let image = EditImage::Reader(&mut reader);
                to_data_url(&image)
            };
            Ok(result.map_or_else(
                |error| json!({ "error": { "status": null, "message": error.to_string() } }),
                |url| json!({ "data_url": url, "position": reader.position() }),
            ))
        }
        Source::Tuple { content } => {
            let inner = EditImage::Bytes(content);
            Ok(data_url_result(to_data_url(&EditImage::Tuple(&inner))))
        }
        Source::Value { value } => Ok(data_url_result(to_data_url(&json_image(value)))),
    }
}

fn prepare_outcome(
    model: &str,
    prompt: Option<&str>,
    image: Option<&PrepareImage>,
    params: &ImageParams,
) -> HelperResult<Value> {
    let Some(image) = image else {
        let images = [None];
        return Ok(prepare_body(
            prepare_form(model, prompt, &images, params),
            None,
        ));
    };
    match image {
        PrepareImage::Bytes { content } => {
            let images = [Some(EditImage::Bytes(content))];
            Ok(prepare_body(
                prepare_form(model, prompt, &images, params),
                None,
            ))
        }
        PrepareImage::Path { name, content } => {
            let path = fixture_path(name, content)?;
            let images = [Some(EditImage::Path(&path))];
            Ok(prepare_body(
                prepare_form(model, prompt, &images, params),
                None,
            ))
        }
        PrepareImage::Reader { content, position } => {
            let mut reader = Cursor::new(content.clone());
            reader.set_position(*position);
            let form = {
                let images = [Some(EditImage::Reader(&mut reader))];
                prepare_form(model, prompt, &images, params)
            };
            let restored = reader.position();
            Ok(prepare_body(form, Some(restored)))
        }
        PrepareImage::Tuple { content } => {
            let inner = EditImage::Bytes(content);
            let images = [Some(EditImage::Tuple(&inner))];
            Ok(prepare_body(
                prepare_form(model, prompt, &images, params),
                None,
            ))
        }
    }
}

fn prepare_form<'value>(
    model: &str,
    prompt: Option<&'value str>,
    images: &'value [Option<EditImage<'value>>],
    params: &'value ImageParams,
) -> Result<ImageEditForm, FalEditError> {
    let request = EditRequest {
        prompt,
        images,
        options: params,
    };
    match for_model(model) {
        Config::Generic => transform_image_edit_request(&request),
        Config::FluxLoraDepth => flux_lora_depth::transform_image_edit_request(&request),
    }
}

fn prepare_body(form: Result<ImageEditForm, FalEditError>, position: Option<u64>) -> Value {
    form.map_or_else(
        |error| json!({ "error": { "status": null, "message": error.to_string() } }),
        |form| {
            json!({
                "body": form.data,
                "files": form.files.len(),
                "position": position,
            })
        },
    )
}

fn map_outcome(model: &str, options: &[ImageParams]) -> Value {
    let catalog = BundledCatalog::default().get();
    let cases: Vec<Value> = options
        .iter()
        .map(|options| match for_model(model) {
            Config::Generic => json!({ "params": map_openai_params(options, model, &catalog) }),
            Config::FluxLoraDepth => {
                json!({ "params": flux_lora_depth::map_openai_params(options) })
            }
        })
        .collect();
    json!({ "supported": supported_params(model), "cases": cases })
}

fn supported_params(model: &str) -> &'static [&'static str] {
    match for_model(model) {
        Config::Generic => &GENERIC_OPENAI_PARAMS,
        Config::FluxLoraDepth => &flux_lora_depth::OPENAI_PARAMS,
    }
}

fn url_outcome(
    environment: &Environment,
    api_bases: &[Option<String>],
    models: &[String],
) -> Value {
    let (api_key, api_base) = Sections::fal_image_environment(environment);
    let settings = FalImageSettings { api_key, api_base };
    let mut urls = Vec::new();
    for explicit in api_bases {
        for model in models {
            let url = match for_model(model) {
                Config::Generic => complete_url(&settings, explicit.as_deref(), model),
                Config::FluxLoraDepth => settings.complete_url(explicit.as_deref(), DEPTH_ENDPOINT),
            };
            urls.push(json!({
                "model": model,
                "api_base": explicit,
                "url": url,
            }));
        }
    }
    json!({ "urls": urls })
}

fn reply_outcome(case: &ReplyCase) -> Value {
    match transform_image_edit_response(case.status, &case.headers, case.body.as_bytes(), 0) {
        Ok(response) => {
            let images: Vec<Value> = response.data.iter().map(image_value).collect();
            json!({ "images": images })
        }
        Err(_) => json!({ "error": { "status": case.status } }),
    }
}

fn image_value(image: &ImageDatum) -> Value {
    let provider_fields = image
        .provider_specific_fields
        .as_ref()
        .map(|fields| json!(fields));
    json!({
        "url": image.url.as_deref(),
        "b64_json": image.b64_json.as_deref(),
        "revised_prompt": image.revised_prompt.as_deref(),
        "provider_specific_fields": provider_fields,
    })
}

fn data_url_result(result: Result<String, FalEditError>) -> Value {
    result.map_or_else(
        |error| json!({ "error": { "status": null, "message": error.to_string() } }),
        |url| json!({ "data_url": url, "position": null }),
    )
}

fn fixture_path(name: &str, content: &[u8]) -> Result<PathBuf, Box<HelperError>> {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, content).map_err(|error| helper_error(error.to_string()))?;
    Ok(path)
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
