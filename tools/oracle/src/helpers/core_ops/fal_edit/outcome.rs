use std::io::Cursor;

use betterlite_config::sections::Sections;
use betterlite_core::api::fal::edit::error::FalEditError;
use betterlite_core::api::fal::edit::flux_lora_depth;
use betterlite_core::api::fal::edit::image_input::{EditImage, to_data_url};
use betterlite_core::api::fal::edit::params::{GENERIC_OPENAI_PARAMS, map_openai_params};
use betterlite_core::api::fal::edit::reply::transform_image_edit_response;
use betterlite_core::api::fal::edit::selector::{Config, for_model};
use betterlite_core::api::fal::edit::transformation::{
    EditRequest, json_image, transform_image_edit_request,
};
use betterlite_core::api::fal::images::settings::FalImageSettings;
use betterlite_models::catalog::BundledCatalog;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::image_edits::request::ImageEditForm;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::input::{Environment, Input, PrepareImage, ReplyCase, Source};
use super::slots::{Sources, collect_slots, fixture_path, reader_positions, request_arrays};

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::ToDataUrl { sources } => {
            let outcomes = sources
                .iter()
                .map(source_outcome)
                .collect::<Result<Vec<Value>, _>>()?;
            Ok(json!({ "sources": outcomes }))
        }
        Input::Prepare {
            model,
            prompt,
            images,
            mask,
            params,
        } => prepare_outcome(model, prompt.as_deref(), images, mask, params),
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

fn source_outcome(source: &Source) -> Result<Value, Box<HelperError>> {
    match source {
        Source::Bytes { content } => {
            Ok(data_url_result(to_data_url(&mut EditImage::Bytes(content))))
        }
        Source::Path { name, content } => {
            let path = fixture_path(name, content)?;
            Ok(data_url_result(to_data_url(&mut EditImage::Path(&path))))
        }
        Source::Reader { content, position } => {
            let mut reader = Cursor::new(content.clone());
            reader.set_position(*position);
            let result = {
                let mut image = EditImage::Reader(&mut reader);
                to_data_url(&mut image)
            };
            Ok(result.map_or_else(
                |error| json!({ "error": { "status": null, "message": error.to_string() } }),
                |url| json!({ "data_url": url, "position": reader.position() }),
            ))
        }
        Source::Tuple { content } => {
            let mut inner = EditImage::Bytes(content);
            Ok(data_url_result(to_data_url(&mut EditImage::Tuple(
                &mut inner,
            ))))
        }
        Source::Value { value } => Ok(data_url_result(to_data_url(&mut json_image(value)))),
    }
}

fn prepare_outcome(
    model: &str,
    prompt: Option<&str>,
    images: &[Option<PrepareImage>],
    mask: &[Option<PrepareImage>],
    params: &ImageParams,
) -> Result<Value, Box<HelperError>> {
    let mut sources = Sources::default();
    let mut readers = Vec::new();
    let image_slots = collect_slots(images, &mut sources, &mut readers)?;
    let mask_slots = collect_slots(mask, &mut sources, &mut readers)?;
    let mut inners: Vec<EditImage<'_>> = sources.inners();
    let form = {
        let (mut image_request, mut mask_request) = request_arrays(
            &sources,
            &mut readers,
            &mut inners,
            &image_slots,
            &mask_slots,
        )?;
        prepare_form(model, prompt, &mut image_request, &mut mask_request, params)
    };
    let (image_positions, mask_positions) = reader_positions(&image_slots, &mask_slots, &readers);
    Ok(prepare_body(form, &image_positions, &mask_positions))
}

fn prepare_form<'value>(
    model: &str,
    prompt: Option<&'value str>,
    images: &'value mut [Option<EditImage<'value>>],
    mask: &'value mut [Option<EditImage<'value>>],
    params: &'value ImageParams,
) -> Result<ImageEditForm, FalEditError> {
    let mut request = EditRequest {
        prompt,
        images,
        mask,
        options: params,
    };
    match for_model(model) {
        Config::Generic => transform_image_edit_request(&mut request),
        Config::FluxLoraDepth => flux_lora_depth::transform_image_edit_request(&mut request),
    }
}

fn prepare_body(
    form: Result<ImageEditForm, FalEditError>,
    positions: &[Option<u64>],
    mask_positions: &[Option<u64>],
) -> Value {
    form.map_or_else(
        |error| json!({ "error": { "status": null, "message": error.to_string() } }),
        |built| {
            json!({
                "body": built.data,
                "positions": positions,
                "mask_positions": mask_positions,
            })
        },
    )
}

fn map_outcome(model: &str, options: &[ImageParams]) -> Value {
    let catalog = BundledCatalog::default().get();
    let cases: Vec<Value> = options
        .iter()
        .map(|entry| match for_model(model) {
            Config::Generic => json!({ "params": map_openai_params(entry, model, &catalog) }),
            Config::FluxLoraDepth => json!({ "params": flux_lora_depth::map_openai_params(entry) }),
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
            let url = for_model(model).complete_url(&settings, explicit.as_deref(), model);
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
        Ok(response) => json!({
            "data": response.data,
            "usage": response.usage,
            "background": response.background,
            "quality": response.quality,
            "output_format": response.output_format,
            "size": response.size,
        }),
        Err(_) => json!({ "error": { "status": case.status } }),
    }
}

fn data_url_result(result: Result<String, FalEditError>) -> Value {
    result.map_or_else(
        |error| json!({ "error": { "status": null, "message": error.to_string() } }),
        |url| json!({ "data_url": url, "position": null }),
    )
}
