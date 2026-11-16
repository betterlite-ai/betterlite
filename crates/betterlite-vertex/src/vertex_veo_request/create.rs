use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use super::error::VeoRequestError;
use super::image::{ImageInput, MimeReader, image_object};
use super::params::MappedParams;

// transform_video_create_request: the mapped options with every extra_body option laid over them
pub struct CreateInput<'media> {
    pub prompt: &'media str,
    pub api_base: &'media str,
    pub mapped: MappedParams<'media>,
    pub extra: &'media Object,
}

#[derive(Debug)]
pub struct CreateForm {
    pub body: Object,
    pub url: String,
}

pub fn create_request(
    input: CreateInput<'_>,
    mime: &dyn MimeReader,
) -> Result<CreateForm, VeoRequestError> {
    let CreateInput {
        prompt,
        api_base,
        mapped,
        extra,
    } = input;
    let (mut options, image) = merged_options(mapped, extra);
    let mut instance = Object::new();
    instance.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
    if merge_instances(&mut instance, &mut options) {
        unconsumed_image(&mut options, image)?;
    } else {
        attach_image(&mut instance, image, &mut options, mime)?;
    }
    Ok(CreateForm {
        body: create_body(instance, options),
        url: format!("{api_base}:predictLongRunning"),
    })
}

// the mapper's options with extra_body laid over them, as LiteLLM merges the two dicts
fn merged_options<'media>(
    mapped: MappedParams<'media>,
    extra: &Object,
) -> (Object, Option<ImageInput<'media>>) {
    let MappedParams {
        image,
        parameters,
        resolution,
        aspect_ratio,
        duration_seconds,
    } = mapped;
    let mut options = mapped_options(parameters, resolution, aspect_ratio, duration_seconds);
    options.extend(extra.clone());
    let image = extra.get("image").cloned().map(ImageInput::Json).or(image);
    (options, image)
}

// the mapper's options as the request dict holds them, before any extra_body option overrides one
fn mapped_options(
    parameters: Option<Object>,
    resolution: Option<Json>,
    aspect_ratio: Option<String>,
    duration_seconds: Option<i64>,
) -> Object {
    let mut options = Object::new();
    if let Some(parameters) = parameters {
        options.insert("parameters".to_owned(), Json::Object(parameters));
    }
    if let Some(resolution) = resolution {
        options.insert("resolution".to_owned(), resolution);
    }
    if let Some(aspect_ratio) = aspect_ratio {
        options.insert("aspectRatio".to_owned(), Json::Text(aspect_ratio));
    }
    if let Some(duration_seconds) = duration_seconds {
        options.insert(
            "durationSeconds".to_owned(),
            Json::Number(Number::from(duration_seconds)),
        );
    }
    options
}

// a full instances dict replaces the prompt and then skips the image branch
fn merge_instances(instance: &mut Object, options: &mut Object) -> bool {
    let supplied = options.get("instances").cloned();
    let Some(Json::Object(instances)) = supplied else {
        return false;
    };
    instance.extend(instances);
    options.shift_remove("instances");
    true
}

// the instances branch skips image conversion, so the image option is flattened like any other
fn unconsumed_image(
    options: &mut Object,
    image: Option<ImageInput<'_>>,
) -> Result<(), VeoRequestError> {
    match image {
        Some(ImageInput::Json(value)) => {
            options.insert("image".to_owned(), value);
            Ok(())
        }
        Some(ImageInput::File(_)) => Err(VeoRequestError::ImageInParameters),
        None => Ok(()),
    }
}

// the image branch skips a null image, which stays among the flat parameters
fn attach_image(
    instance: &mut Object,
    image: Option<ImageInput<'_>>,
    options: &mut Object,
    mime: &dyn MimeReader,
) -> Result<(), VeoRequestError> {
    let Some(image) = image else {
        return Ok(());
    };
    if let ImageInput::Json(Json::Null) = &image {
        options.insert("image".to_owned(), Json::Null);
        return Ok(());
    }
    instance.insert("image".to_owned(), Json::Object(image_object(image, mime)?));
    options.shift_remove("image");
    Ok(())
}

fn create_body(instance: Object, options: Object) -> Object {
    let mut body = Object::new();
    body.insert(
        "instances".to_owned(),
        Json::List(vec![Json::Object(instance)]),
    );
    let vertex = flatten_parameters(options);
    if !vertex.is_empty() {
        body.insert("parameters".to_owned(), Json::Object(vertex));
    }
    body
}

// the nested parameters block is flattened first so every remaining option overrides it
fn flatten_parameters(mut options: Object) -> Object {
    let mut vertex = match options.shift_remove("parameters") {
        Some(Json::Object(nested)) => nested,
        _ => Object::new(),
    };
    vertex.extend(options);
    vertex
}
