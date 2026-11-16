mod tests;

use std::collections::BTreeMap;
use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::studio_veo_request::create::{CreateInput, create_request};
use betterlite_vertex::studio_veo_request::environment::{KeySources, validate_environment};
use betterlite_vertex::studio_veo_request::error::StudioVeoError;
use betterlite_vertex::studio_veo_request::params::{MapInput, Reference, map_openai_params};
use betterlite_vertex::studio_veo_request::url::complete_url;
use betterlite_vertex::vertex_veo_request::image::{B1120Mime, ImageInput};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    requests: Vec<Request>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    model: String,
    prompt: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    input_reference: Option<ImageSeed>,
    #[serde(default)]
    image: Option<ImageSeed>,
    #[serde(default)]
    params: Object,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    generic_key: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    params_api_key: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

// a media option arrives as JSON, as a file with its cursor position, or as an explicit null
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ImageSeed {
    Null,
    Dict {
        value: Object,
    },
    File {
        data: String,
        #[serde(default)]
        position: u64,
    },
    Text {
        value: String,
    },
}

enum PreparedImage {
    Json(Json),
    File(Cursor<Vec<u8>>),
}

impl PreparedImage {
    fn input(&mut self) -> ImageInput<'_> {
        match self {
            Self::Json(value) => ImageInput::Json(value.clone()),
            Self::File(cursor) => ImageInput::File(cursor),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input
            .requests
            .iter()
            .map(answer)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "requests": answers }))
    }
}

fn answer(request: &Request) -> Result<Value, Box<HelperError>> {
    let settings = settings(request);
    let mut reference = prepare(request.input_reference.as_ref())?;
    let mut image = prepare(request.image.as_ref())?;
    let reference_input = match (&request.input_reference, reference.as_mut()) {
        (Some(ImageSeed::Null), _) => Reference::Null,
        (Some(_), Some(prepared)) => Reference::Image(prepared.input()),
        _ => Reference::Absent,
    };
    let image_input = image.as_mut().map(PreparedImage::input);
    let mut params = request.params.clone();
    let size = text_option(&mut params, "size")?;
    let seconds = text_option(&mut params, "seconds")?;
    let resolution = params.shift_remove("resolution");
    let mapped = map_openai_params(MapInput {
        input_reference: reference_input,
        image: image_input,
        size,
        seconds,
        resolution,
        options: params,
    });
    let url = complete_url(&request.model, request.api_base.as_deref(), &settings);
    let headers = outgoing_headers(request, &settings).or_value_error()?;
    let form = create_request(
        CreateInput {
            prompt: &request.prompt,
            api_base: &url,
            mapped,
        },
        &B1120Mime,
    )
    .or_value_error()?;
    let revealed: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().clone()))
        .collect();
    let position = chosen_position(request, reference.as_ref(), image.as_ref());
    match position {
        Some(offset) => Ok(json!({
            "url": form.url,
            "headers": revealed,
            "body": form.body,
            "image_position": offset,
        })),
        None => Ok(json!({
            "url": form.url,
            "headers": revealed,
            "body": form.body,
        })),
    }
}

fn settings(request: &Request) -> Settings {
    let configured = request
        .env
        .get("GEMINI_API_BASE")
        .filter(|value| !value.is_empty());
    Settings {
        environment_key: environment_key(&request.env),
        generic_api_key: request.generic_key.as_deref().map(SecretString::from),
        api_base: configured
            .cloned()
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
    }
}

fn environment_key(environment: &BTreeMap<String, String>) -> Option<SecretString> {
    ["GOOGLE_API_KEY", "GEMINI_API_KEY"]
        .into_iter()
        .find_map(|name| environment.get(name).filter(|value| !value.is_empty()))
        .map(|value| SecretString::from(value.as_str()))
}

fn outgoing_headers(
    request: &Request,
    settings: &Settings,
) -> Result<BTreeMap<String, SecretString>, StudioVeoError> {
    let api_key = request.api_key.as_deref().map(SecretString::from);
    let params_api_key = request.params_api_key.as_deref().map(SecretString::from);
    validate_environment(
        &request.headers,
        KeySources {
            api_key: api_key.as_ref(),
            litellm_params_api_key: params_api_key.as_ref(),
        },
        settings,
    )
}

fn prepare(seed: Option<&ImageSeed>) -> Result<Option<PreparedImage>, Box<HelperError>> {
    match seed {
        None | Some(ImageSeed::Null) => Ok(None),
        Some(ImageSeed::Dict { value }) => {
            Ok(Some(PreparedImage::Json(Json::Object(value.clone()))))
        }
        Some(ImageSeed::Text { value }) => Ok(Some(PreparedImage::Json(Json::Text(value.clone())))),
        Some(ImageSeed::File { data, position }) => {
            let mut cursor = Cursor::new(STANDARD.decode(data).or_config_error()?);
            cursor.set_position(*position);
            Ok(Some(PreparedImage::File(cursor)))
        }
    }
}

fn text_option(options: &mut Object, name: &str) -> Result<Option<String>, Box<HelperError>> {
    match options.shift_remove(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(format!("{name} must be a string")).or_config_error()
        }
    }
}

fn chosen_position(
    request: &Request,
    reference: Option<&PreparedImage>,
    image: Option<&PreparedImage>,
) -> Option<u64> {
    let chosen = match &request.input_reference {
        Some(ImageSeed::Null) => return None,
        Some(_) => reference,
        None => image,
    };
    match chosen {
        Some(PreparedImage::File(cursor)) => Some(cursor.position()),
        Some(PreparedImage::Json(_)) | None => None,
    }
}
