use betterlite_anthropic::credentials::{
    ANTHROPIC_VERSION, AnthropicSettings, get_auth_header, handle_anthropic_oauth,
};
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::MessagesPrepareError;
use crate::api::anthropic_beta_filter::native_injection::{NativeRequest, inject_native_betas};

pub const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
pub const CONTENT_TYPE_HEADER: &str = "content-type";
pub const JSON_CONTENT_TYPE: &str = "application/json";

// LiteLLM hands the beta updater its default provider, and the returned api_base is the supplied value unchanged
pub fn validate_environment(
    headers: &mut IndexMap<String, String>,
    api_key: Option<SecretString>,
    api_base: Option<&str>,
    settings: &AnthropicSettings,
) -> Result<Option<String>, MessagesPrepareError> {
    let resolved_key = handle_anthropic_oauth(headers, api_key);
    let has_credential = headers.keys().any(|name| {
        let lowered = name.to_lowercase();
        lowered == "x-api-key" || lowered == "authorization"
    });
    if !has_credential {
        let header = get_auth_header(resolved_key.as_ref(), None, false, settings)
            .ok_or(MessagesPrepareError::MissingCredential)?;
        headers.insert(
            header.name.to_owned(),
            header.value.expose_secret().to_owned(),
        );
    }
    if !headers.contains_key(ANTHROPIC_VERSION_HEADER) {
        headers.insert(
            ANTHROPIC_VERSION_HEADER.to_owned(),
            ANTHROPIC_VERSION.to_owned(),
        );
    }
    if !headers.contains_key(CONTENT_TYPE_HEADER) {
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
    }
    let optional_params = Object::new();
    let messages: [Json; 0] = [];
    let request = NativeRequest {
        provider: "anthropic",
        optional_params: &optional_params,
        messages: &messages,
    };
    *headers = inject_native_betas(headers.clone(), &request)?;
    Ok(api_base.map(str::to_owned))
}
