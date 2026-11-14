use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

use super::error::CatoError;
use super::{CallHook, CatoGuardrail, LITELLM_VERSION};

const AUTHORIZATION_HEADER: &str = "authorization";
const CALL_ID_HEADER: &str = "x-cato-call-id";
const GATEWAY_KEY_ALIAS_HEADER: &str = "x-cato-gateway-key-alias";
const HOOK_HEADER: &str = "x-cato-litellm-hook";
const USER_EMAIL_HEADER: &str = "x-cato-user-email";
const VERSION_HEADER: &str = "x-cato-litellm-version";

// LiteLLM's _build_cato_headers: the optional headers are left out when the value is absent
pub(super) fn build_cato_headers(
    guardrail: &CatoGuardrail,
    hook: CallHook,
    key_alias: Option<&str>,
    user_email: Option<&str>,
    litellm_call_id: Option<&str>,
) -> Result<HeaderMap, CatoError> {
    let mut headers = HeaderMap::new();
    insert(
        &mut headers,
        AUTHORIZATION_HEADER,
        &format!("Bearer {}", guardrail.api_key().expose_secret()),
        true,
    )?;
    insert(&mut headers, HOOK_HEADER, hook.header_value(), false)?;
    insert(&mut headers, VERSION_HEADER, LITELLM_VERSION, false)?;
    if let Some(call_id) = litellm_call_id {
        insert(&mut headers, CALL_ID_HEADER, call_id, false)?;
    }
    if let Some(email) = user_email {
        insert(&mut headers, USER_EMAIL_HEADER, email, true)?;
    }
    if let Some(alias) = key_alias {
        insert(&mut headers, GATEWAY_KEY_ALIAS_HEADER, alias, false)?;
    }
    Ok(headers)
}

fn insert(
    headers: &mut HeaderMap,
    name: &'static str,
    value: &str,
    sensitive: bool,
) -> Result<(), CatoError> {
    let mut header_value =
        HeaderValue::from_str(value).map_err(|source| CatoError::InvalidHeader { name, source })?;
    header_value.set_sensitive(sensitive);
    headers.insert(HeaderName::from_static(name), header_value);
    Ok(())
}
