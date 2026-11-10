use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::gdc_settings::{BoolOrString, CallSettings, GlobalSettings};
use betterlite_env::flag;
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, ChatContentError, Mode, UrlDownloader};
use crate::openai_base_config::{CatalogCapabilities, OpenAiConfig};

mod tests;

pub const SUPPORTS_VERTEX_PARAMS: bool = true;

const PROVIDER: &str = "gdc";
const PROJECTS_PATH: &str = "/v1/projects/";
const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";
const VERTEX_PARAMS: [&str; 2] = ["vertex_project", "vertex_location"];
const ROUTING_PARAMS: [&str; 6] = [
    "vertex_project",
    "vertex_ai_project",
    "vertex_location",
    "vertex_ai_location",
    "ssl_verify",
    "gdc_token_caching",
];
const PROJECT_SOURCES: [&str; 2] = ["vertex_project", "vertex_ai_project"];
const LOCATION_SOURCES: [&str; 2] = ["vertex_location", "vertex_ai_location"];

#[derive(Debug, Error)]
pub enum GdcPolicyError {
    #[error("{message}")]
    Authentication { message: String, model: String },
    #[error("the GDC Gemini {param} must be a string")]
    Parameter { param: String },
}

impl GdcPolicyError {
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Authentication { message, model } => {
                ErrorClass::Authentication(Box::new(ResponseDetails::from(ErrorContext {
                    message: message.clone(),
                    model: Some(model.clone()),
                    provider: Some(PROVIDER.to_owned()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                })))
            }
            Self::Parameter { param } => {
                ErrorClass::BadRequest(Box::new(ResponseDetails::from(ErrorContext {
                    message: format!("the GDC Gemini {param} must be a string"),
                    model: None,
                    provider: Some(PROVIDER.to_owned()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                })))
            }
        }
    }
}

#[must_use]
pub fn get_supported_openai_params(
    model: &str,
    config: &OpenAiConfig,
    catalog: &CatalogCapabilities,
) -> Vec<&'static str> {
    let mut params = VERTEX_PARAMS.to_vec();
    params.extend(config.supported_params(model, catalog));
    params
}

pub fn resolve_project(
    call: &CallSettings,
    global: &GlobalSettings,
    optional: &Object,
) -> Result<Option<String>, GdcPolicyError> {
    let configured = [
        call.vertex_project.as_deref(),
        call.vertex_ai_project.as_deref(),
        global.vertex_project.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.is_empty());
    if let Some(project) = configured {
        return Ok(Some(project.to_owned()));
    }
    for name in PROJECT_SOURCES {
        if let Some(project) = request_text(optional.get(name), name)? {
            return Ok(Some(project.to_owned()));
        }
    }
    Ok(None)
}

pub fn resolve_location(
    call: &CallSettings,
    global: &GlobalSettings,
    optional: &Object,
) -> Result<Option<String>, GdcPolicyError> {
    let configured = [
        call.vertex_location.as_deref(),
        call.vertex_ai_location.as_deref(),
        global.vertex_location.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.is_empty());
    if let Some(location) = configured {
        return Ok(Some(location.to_owned()));
    }
    for name in LOCATION_SOURCES {
        if let Some(location) = request_text(optional.get(name), name)? {
            return Ok(Some(location.to_owned()));
        }
    }
    Ok(None)
}

pub fn effective_project(
    api_base: &str,
    call: &CallSettings,
    global: &GlobalSettings,
    optional: &Object,
) -> Result<Option<String>, GdcPolicyError> {
    if let Some(project) = project_from_base(api_base) {
        return Ok(Some(project));
    }
    resolve_project(call, global, optional)
}

pub fn validate_path_id<'value>(
    value: &'value str,
    field: &str,
    model: &str,
) -> Result<&'value str, GdcPolicyError> {
    let checked = value.strip_suffix('\n').unwrap_or(value);
    if !checked.is_empty()
        && checked
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Ok(value);
    }
    Err(GdcPolicyError::Authentication {
        message: format!("the GDC Gemini {field} is not a plain identifier"),
        model: model.to_owned(),
    })
}

pub fn complete_url(
    api_base: Option<&str>,
    call: &CallSettings,
    global: &GlobalSettings,
    optional: &Object,
    model: &str,
) -> Result<String, GdcPolicyError> {
    let Some(base) = [
        api_base,
        global.gdc_api_base.as_deref(),
        global.api_base.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|base| !base.is_empty()) else {
        return Err(GdcPolicyError::Authentication {
            message: "the GDC Gemini base URL is not set".to_owned(),
            model: model.to_owned(),
        });
    };
    let prefixed = with_scheme(base);
    let trimmed = prefixed.trim_end_matches('/');
    if trimmed.contains(PROJECTS_PATH) {
        return Ok(trimmed.to_owned());
    }
    let Some(project) = resolve_project(call, global, optional)? else {
        return Err(GdcPolicyError::Authentication {
            message: "the GDC Gemini project is not set".to_owned(),
            model: model.to_owned(),
        });
    };
    let Some(location) = resolve_location(call, global, optional)? else {
        return Err(GdcPolicyError::Authentication {
            message: "the GDC Gemini location is not set".to_owned(),
            model: model.to_owned(),
        });
    };
    let checked_project = validate_path_id(&project, "vertex_project", model)?;
    let checked_location = validate_path_id(&location, "vertex_location", model)?;
    Ok(format!(
        "{trimmed}{PROJECTS_PATH}{checked_project}/locations/{checked_location}{CHAT_COMPLETIONS_PATH}"
    ))
}

#[must_use]
pub fn audience(api_base: &str) -> String {
    let url = with_scheme(api_base);
    // urlsplit strips leading C0 controls and spaces, then removes tab, carriage return and newline anywhere
    let cleaned = url
        .trim_start_matches(|character: char| character <= '\u{20}')
        .replace(['\t', '\r', '\n'], "");
    let (scheme, rest) = split_scheme(&cleaned);
    let netloc = rest
        .strip_prefix("//")
        .and_then(|authority| authority.split(['/', '?', '#']).next())
        .unwrap_or_default();
    format!("{}://{netloc}", scheme.unwrap_or_default())
}

#[must_use]
pub fn read_env_bool(
    setting: &str,
    value: Option<&BoolOrString>,
    environment: Option<&str>,
    default: bool,
) -> BoolOrString {
    match value {
        Some(BoolOrString::Bool(configured)) => BoolOrString::Bool(*configured),
        Some(BoolOrString::Text(text)) => match flag(setting, text) {
            Ok(Some(parsed)) => BoolOrString::Bool(parsed),
            Ok(None) | Err(_) => BoolOrString::Text(text.clone()),
        },
        None => environment.map_or(BoolOrString::Bool(default), |text| {
            match flag(setting, text) {
                Ok(Some(parsed)) => BoolOrString::Bool(parsed),
                Ok(None) | Err(_) => BoolOrString::Bool(default),
            }
        }),
    }
}

#[must_use]
pub const fn caching_enabled(setting: &BoolOrString) -> bool {
    match setting {
        BoolOrString::Bool(configured) => *configured,
        BoolOrString::Text(text) => !text.is_empty(),
    }
}

pub struct TransformRequest<'call> {
    pub model: &'call str,
    pub messages: &'call [ChatMessage],
    pub optional_params: &'call Object,
    pub environment: &'call dyn EnvironmentLookup,
    pub downloads: &'call dyn UrlDownloader,
}

pub fn transform_request(request: &TransformRequest<'_>) -> Result<Object, ChatContentError> {
    let content = ChatContent {
        model: request.model.strip_prefix("gdc/").unwrap_or(request.model),
        messages: request.messages,
        optional_params: request.optional_params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: request.environment,
        downloads: request.downloads,
    };
    let mut body = content.transform(Mode::Sync)?.to_body()?;
    for name in ROUTING_PARAMS {
        body.shift_remove(name);
    }
    Ok(body)
}

fn with_scheme(base: &str) -> String {
    if base.starts_with("http") {
        base.to_owned()
    } else {
        format!("https://{base}")
    }
}

fn request_text<'value>(
    value: Option<&'value Json>,
    param: &str,
) -> Result<Option<&'value str>, GdcPolicyError> {
    let Some(present) = value else {
        return Ok(None);
    };
    if !present.is_truthy() {
        return Ok(None);
    }
    present.as_str().map_or_else(
        || {
            Err(GdcPolicyError::Parameter {
                param: param.to_owned(),
            })
        },
        |text| Ok(Some(text)),
    )
}

fn project_from_base(api_base: &str) -> Option<String> {
    let mut search_start = 0;
    while let Some(found) = api_base.get(search_start..)?.find(PROJECTS_PATH) {
        let project_start = search_start
            .checked_add(found)?
            .checked_add(PROJECTS_PATH.len())?;
        let tail = api_base.get(project_start..)?;
        let first = tail.chars().next()?;
        if first != '/' {
            let end = tail.find('/').unwrap_or(tail.len());
            return tail.get(..end).map(str::to_owned);
        }
        search_start = project_start;
    }
    None
}

fn split_scheme(url: &str) -> (Option<String>, &str) {
    let Some((candidate, rest)) = url.split_once(':') else {
        return (None, url);
    };
    let mut characters = candidate.chars();
    let valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        });
    if valid {
        return (Some(candidate.to_lowercase()), rest);
    }
    (None, url)
}
