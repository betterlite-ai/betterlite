use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::gdc_settings::{BoolOrString, CallSettings, GlobalSettings};
use betterlite_env::flag;
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::provider_defaults_a::url;
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
    #[error("the GDC Gemini base URL cannot be split")]
    InvalidBase,
}

impl GdcPolicyError {
    #[must_use]
    #[rustfmt::skip]
    pub fn class(&self) -> ErrorClass {
        let model = match self {
            Self::Missing { model, .. } | Self::Identifier { model, .. } => Some(model.clone()),
            Self::Parameter { .. } | Self::InvalidBase => None,
        };
        let context = ErrorContext { message: self.to_string(), model, provider: Some(PROVIDER.to_owned()), max_retries: None, num_retries: None, body_type: None, body_param: None, body_code: None };
        match self {
            Self::Missing { .. } | Self::Identifier { .. } => ErrorClass::Authentication(Box::new(ResponseDetails::from(context))),
            Self::Parameter { .. } => ErrorClass::BadRequest(Box::new(ResponseDetails::from(context))),
            Self::InvalidBase => ErrorClass::ApiConnection(Box::new(context)),
        }
    }
}

#[must_use]
#[rustfmt::skip]
pub fn get_supported_openai_params(model: &str, config: &OpenAiConfig, catalog: &CatalogCapabilities) -> Vec<&'static str> {
    let mut params = if SUPPORTS_VERTEX_PARAMS { VERTEX_PARAMS.to_vec() } else { Vec::new() };
    params.extend(config.supported_params(model, catalog));
    params
}

#[must_use]
#[rustfmt::skip]
pub fn resolve_project(call: &CallSettings, global: &GlobalSettings, optional: &Object) -> Option<Json> {
    resolve(
        &[call.vertex_project.as_deref(), call.vertex_ai_project.as_deref(), global.vertex_project.as_deref()],
        optional,
        PROJECT_SOURCES,
    )
}

#[must_use]
#[rustfmt::skip]
pub fn resolve_location(call: &CallSettings, global: &GlobalSettings, optional: &Object) -> Option<Json> {
    resolve(
        &[call.vertex_location.as_deref(), call.vertex_ai_location.as_deref(), global.vertex_location.as_deref()],
        optional,
        LOCATION_SOURCES,
    )
}

#[rustfmt::skip]
fn resolve(configured: &[Option<&str>], optional: &Object, names: [&'static str; 2]) -> Option<Json> {
    if let Some(value) = configured.iter().flatten().find(|value| !value.is_empty()) {
        return Some(Json::Text((*value).to_owned()));
    }
    names.into_iter().find_map(|name| optional.get(name).filter(|value| value.is_truthy()).cloned())
}

#[must_use]
#[rustfmt::skip]
pub fn effective_project(api_base: &str, call: &CallSettings, global: &GlobalSettings, optional: &Object) -> Option<Json> {
    if let Some(project) = project_from_base(api_base) {
        return Some(Json::Text(project));
    }
    resolve_project(call, global, optional)
}

#[rustfmt::skip]
pub fn validate_path_id<'value>(value: &'value Json, field: &'static str, model: &str) -> Result<&'value str, GdcPolicyError> {
    let Some(text) = value.as_str() else {
        return Err(GdcPolicyError::Parameter { param: field });
    };
    let checked = text.strip_suffix('\n').unwrap_or(text);
    let plain = !checked.is_empty() && checked.chars().all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if plain {
        return Ok(text);
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
    let Some(project) = resolve_project(call, global, optional) else {
        return Err(GdcPolicyError::Missing { what: "project", model: model.to_owned() });
    };
    let Some(location) = resolve_location(call, global, optional) else {
        return Err(GdcPolicyError::Missing { what: "location", model: model.to_owned() });
    };
    let checked_project = validate_path_id(&project, "vertex_project", model)?;
    let checked_location = validate_path_id(&location, "vertex_location", model)?;
    Ok(format!("{trimmed}{PROJECTS_PATH}{checked_project}/locations/{checked_location}{COMPLETIONS_PATH}"))
}

#[rustfmt::skip]
pub fn audience(api_base: &str) -> Result<String, GdcPolicyError> {
    let Ok(parts) = url::parse(&with_scheme(api_base)) else {
        return Err(GdcPolicyError::InvalidBase);
    };
    let hostinfo = parts.netloc.rsplit_once('@').map_or(parts.netloc.as_str(), |(_, after)| after);
    // urlsplit runs its bracketed-host check on the host after the userinfo even when no bracket follows it
    if parts.netloc.contains('[') && !hostinfo.contains('[') {
        let host = hostinfo.split_once(':').map_or(hostinfo, |(host, _)| host);
        if !bracketed_host_is_valid(host) {
            return Err(GdcPolicyError::InvalidBase);
        }
    }
    Ok(format!("{}://{}", parts.scheme, parts.netloc))
}

// urlsplit's userinfo-only branch cuts the host at ':', so only an IPvFuture literal can pass here
#[rustfmt::skip]
fn bracketed_host_is_valid(host: &str) -> bool {
    let Some((hex, suffix)) = host.strip_prefix('v').and_then(|future| future.split_once('.')) else {
        return false;
    };
    !hex.is_empty() && hex.chars().all(|character| character.is_ascii_hexdigit()) && !suffix.is_empty()
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
fn project_from_base(api_base: &str) -> Option<String> {
    api_base.split(PROJECTS_PATH).skip(1).find_map(|tail| {
        let end = tail.find('/').unwrap_or(tail.len());
        tail.get(..end).filter(|project| !project.is_empty()).map(str::to_owned)
    })
}
