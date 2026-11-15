use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use crate::endpoint_target::headers::set_headers;
use crate::endpoint_target::target::AuthHeader;

use super::error::GeminiHttpError;

// LiteLLM's validate_environment: the content type, then the auth header pair, then the caller's headers last
#[must_use]
pub fn request_headers(
    auth: Option<&AuthHeader>,
    caller: &BTreeMap<String, String>,
) -> BTreeMap<String, SecretString> {
    match auth {
        Some(AuthHeader::Headers(values)) => {
            // the auth map lands between the forced content type and the caller's headers
            let mut headers = set_headers(None, &BTreeMap::new());
            for (name, value) in values {
                headers.insert(name.clone(), value.clone());
            }
            for (name, value) in caller {
                headers.insert(name.clone(), SecretString::from(value.clone()));
            }
            headers
        }
        Some(AuthHeader::Token(token)) => set_headers(Some(token), caller),
        None => set_headers(None, caller),
    }
}

// LiteLLM reads use_psc_endpoint_format from the optional params, absent or null taking the default false
pub fn psc_endpoint_format(optional_params: &Object) -> Result<bool, GeminiHttpError> {
    match optional_params.get("use_psc_endpoint_format") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(flag)) => Ok(*flag),
        Some(_) => Err(GeminiHttpError::Flag {
            field: "use_psc_endpoint_format",
        }),
    }
}
