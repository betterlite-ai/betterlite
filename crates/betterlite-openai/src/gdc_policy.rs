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
const COMPLETIONS_PATH: &str = "/chat/completions";
#[rustfmt::skip]
const VERTEX_PARAMS: [&str; 2] = ["vertex_project", "vertex_location"];
#[rustfmt::skip]
const ROUTING_PARAMS: [&str; 6] = ["vertex_project", "vertex_ai_project", "vertex_location", "vertex_ai_location", "ssl_verify", "gdc_token_caching"];
#[rustfmt::skip]
const PROJECT_SOURCES: [&str; 2] = ["vertex_project", "vertex_ai_project"];
#[rustfmt::skip]
const LOCATION_SOURCES: [&str; 2] = ["vertex_location", "vertex_ai_location"];

#[derive(Debug, Error)]
pub enum GdcPolicyError {
    #[error("the GDC Gemini {what} is not set")]
    Missing { what: &'static str, model: String },
    #[error("the GDC Gemini {field} is not a plain identifier")]
    Identifier { field: &'static str, model: String },
    #[error("the GDC Gemini {param} must be a string")]
    Parameter { param: &'static str },
}

impl GdcPolicyError {
    #[must_use]
    #[rustfmt::skip]
    pub fn class(&self) -> ErrorClass {
        let (message, model, status) = match self {
            Self::Missing { what, model } => (format!("the GDC Gemini {what} is not set"), Some(model.clone()), 401_u16),
            Self::Identifier { field, model } => (format!("the GDC Gemini {field} is not a plain identifier"), Some(model.clone()), 401_u16),
            Self::Parameter { param } => (format!("the GDC Gemini {param} must be a string"), None, 400_u16),
        };
        let context = ErrorContext { message, model, provider: Some(PROVIDER.to_owned()), max_retries: None, num_retries: None, body_type: None, body_param: None, body_code: None };
        match status {
            401 => ErrorClass::Authentication(Box::new(ResponseDetails::from(context))),
            _ => ErrorClass::BadRequest(Box::new(ResponseDetails::from(context))),
        }
    }
}

#[must_use]
#[rustfmt::skip]
pub fn get_supported_openai_params(model: &str, config: &OpenAiConfig, catalog: &CatalogCapabilities) -> Vec<&'static str> {
    let mut params = VERTEX_PARAMS.to_vec();
    params.extend(config.supported_params(model, catalog));
    params
}

#[rustfmt::skip]
pub fn resolve_project(call: &CallSettings, global: &GlobalSettings, optional: &Object) -> Result<Option<String>, GdcPolicyError> {
    resolve(
        &[call.vertex_project.as_deref(), call.vertex_ai_project.as_deref(), global.vertex_project.as_deref()],
        optional,
        PROJECT_SOURCES,
    )
}

#[rustfmt::skip]
pub fn resolve_location(call: &CallSettings, global: &GlobalSettings, optional: &Object) -> Result<Option<String>, GdcPolicyError> {
    resolve(
        &[call.vertex_location.as_deref(), call.vertex_ai_location.as_deref(), global.vertex_location.as_deref()],
        optional,
        LOCATION_SOURCES,
    )
}

#[rustfmt::skip]
fn resolve(configured: &[Option<&str>], optional: &Object, names: [&'static str; 2]) -> Result<Option<String>, GdcPolicyError> {
    if let Some(value) = configured.iter().flatten().find(|value| !value.is_empty()) {
        return Ok(Some((*value).to_owned()));
    }
    for name in names {
        if let Some(value) = request_text(optional.get(name), name)? {
            return Ok(Some(value.to_owned()));
        }
    }
    Ok(None)
}

#[rustfmt::skip]
pub fn effective_project(api_base: &str, call: &CallSettings, global: &GlobalSettings, optional: &Object) -> Result<Option<String>, GdcPolicyError> {
    if let Some(project) = project_from_base(api_base) {
        return Ok(Some(project));
    }
    resolve_project(call, global, optional)
}

#[rustfmt::skip]
pub fn validate_path_id<'value>(value: &'value str, field: &'static str, model: &str) -> Result<&'value str, GdcPolicyError> {
    let checked = value.strip_suffix('\n').unwrap_or(value);
    let plain = !checked.is_empty() && checked.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if plain {
        return Ok(value);
    }
    Err(GdcPolicyError::Identifier { field, model: model.to_owned() })
}

#[rustfmt::skip]
pub fn complete_url(api_base: Option<&str>, call: &CallSettings, global: &GlobalSettings, optional: &Object, model: &str) -> Result<String, GdcPolicyError> {
    let Some(base) = [api_base, global.gdc_api_base.as_deref(), global.api_base.as_deref()].into_iter().flatten().find(|base| !base.is_empty()) else {
        return Err(GdcPolicyError::Missing { what: "base URL", model: model.to_owned() });
    };
    let prefixed = with_scheme(base);
    let trimmed = prefixed.trim_end_matches('/');
    if trimmed.contains(PROJECTS_PATH) {
        return Ok(trimmed.to_owned());
    }
    let Some(project) = resolve_project(call, global, optional)? else {
        return Err(GdcPolicyError::Missing { what: "project", model: model.to_owned() });
    };
    let Some(location) = resolve_location(call, global, optional)? else {
        return Err(GdcPolicyError::Missing { what: "location", model: model.to_owned() });
    };
    let checked_project = validate_path_id(&project, "vertex_project", model)?;
    let checked_location = validate_path_id(&location, "vertex_location", model)?;
    Ok(format!("{trimmed}{PROJECTS_PATH}{checked_project}/locations/{checked_location}{COMPLETIONS_PATH}"))
}

#[must_use]
#[rustfmt::skip]
pub fn audience(api_base: &str) -> String {
    // urlsplit strips leading C0 controls and spaces, then removes tab, carriage return and newline anywhere
    let cleaned = with_scheme(api_base).trim_start_matches(|character: char| character <= '\u{20}').replace(['\t', '\r', '\n'], "");
    let (scheme, rest) = split_scheme(&cleaned);
    let netloc = rest.strip_prefix("//").and_then(|authority| authority.split(['/', '?', '#']).next()).unwrap_or_default();
    format!("{}://{netloc}", scheme.unwrap_or_default())
}

#[must_use]
#[rustfmt::skip]
pub fn read_env_bool(setting: &str, value: Option<&BoolOrString>, environment: Option<&str>, default: bool) -> BoolOrString {
    match value {
        Some(BoolOrString::Bool(configured)) => BoolOrString::Bool(*configured),
        Some(BoolOrString::Text(text)) => flag(setting, text).ok().flatten().map_or_else(|| BoolOrString::Text(text.clone()), BoolOrString::Bool),
        None => environment.map_or(BoolOrString::Bool(default), |text| match flag(setting, text).ok().flatten() {
            Some(parsed) => BoolOrString::Bool(parsed),
            None if text.trim().is_empty() => BoolOrString::Bool(default),
            None => BoolOrString::Text(text.to_owned()),
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

#[rustfmt::skip]
pub fn transform_request(model: &str, messages: &[ChatMessage], optional_params: &Object, environment: &dyn EnvironmentLookup, downloads: &dyn UrlDownloader) -> Result<Object, ChatContentError> {
    let content = ChatContent { model: model.strip_prefix("gdc/").unwrap_or(model), messages, optional_params, provider: Some(PROVIDER), api_base: None, global_api_base: None, openai_system_messages_first: false, environment, downloads };
    let mut body = content.transform(Mode::Sync)?.to_body()?;
    for name in ROUTING_PARAMS {
        body.shift_remove(name);
    }
    Ok(body)
}

#[rustfmt::skip]
fn with_scheme(base: &str) -> String {
    if base.starts_with("http") { base.to_owned() } else { format!("https://{base}") }
}

#[rustfmt::skip]
fn request_text<'value>(value: Option<&'value Json>, param: &'static str) -> Result<Option<&'value str>, GdcPolicyError> {
    let Some(present) = value else { return Ok(None) };
    if !present.is_truthy() {
        return Ok(None);
    }
    present.as_str().map_or_else(|| Err(GdcPolicyError::Parameter { param }), |text| Ok(Some(text)))
}

#[rustfmt::skip]
fn project_from_base(api_base: &str) -> Option<String> {
    api_base.split(PROJECTS_PATH).skip(1).find_map(|tail| {
        let end = tail.find('/').unwrap_or(tail.len());
        tail.get(..end).filter(|project| !project.is_empty()).map(str::to_owned)
    })
}

#[rustfmt::skip]
fn split_scheme(url: &str) -> (Option<String>, &str) {
    let Some((candidate, rest)) = url.split_once(':') else {
        return (None, url);
    };
    let mut characters = candidate.chars();
    let valid = characters.next().is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'));
    if valid { (Some(candidate.to_lowercase()), rest) } else { (None, url) }
}
