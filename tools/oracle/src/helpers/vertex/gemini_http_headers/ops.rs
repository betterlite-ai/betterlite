use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::endpoint_target::target::AuthHeader;
use betterlite_vertex::gemini_http::headers::{psc_endpoint_format, request_headers};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) api_key: Option<String>,
    pub(crate) auth_map: Option<BTreeMap<String, String>>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) optional_params: Object,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let auth = match (&input.auth_map, &input.api_key) {
        (Some(pairs), _) => Some(AuthHeader::Headers(
            pairs
                .iter()
                .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
                .collect(),
        )),
        (None, Some(key)) => Some(AuthHeader::Token(SecretString::from(key.clone()))),
        (None, None) => None,
    };
    let caller = input.headers.clone().unwrap_or_default();
    let headers = request_headers(auth.as_ref(), &caller);
    let exposed: BTreeMap<&str, &str> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    let psc = psc_endpoint_format(&input.optional_params).or_value_error()?;
    Ok(json!({"headers": exposed, "use_psc_endpoint_format": psc}))
}
