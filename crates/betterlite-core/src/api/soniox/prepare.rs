pub mod body;
pub mod decode;
pub mod error;
pub mod logging;
pub mod options;
pub mod raise;
pub mod response;

mod tests;

use std::collections::BTreeMap;

use betterlite_config::types::GeneralSettings;
use betterlite_openai::openai_base_config::{Headers, Object};

use self::error::PrepareError;
use self::options::{HandlerOptions, extract_options};
use crate::api::soniox::records::auth::{resolve_api_base, validate_environment};

pub struct PrepareInput<'call> {
    pub optional_params: &'call Object,
    pub headers: Headers,
    pub api_key: Option<&'call str>,
    pub api_base: Option<&'call str>,
    pub environment: &'call BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Prepared {
    pub auth_headers: Headers,
    pub api_base: String,
    pub params: Object,
    pub options: HandlerOptions,
}

pub fn prepare(input: &PrepareInput<'_>) -> Result<Prepared, PrepareError> {
    let (environment_key, environment_base) =
        GeneralSettings::soniox_record_settings(input.environment);
    let auth_headers = validate_environment(
        input.headers.clone(),
        input.api_key,
        environment_key.as_ref(),
    )?;
    let api_base = resolve_api_base(input.api_base, &environment_base);
    let mut params = input.optional_params.clone();
    let options = extract_options(&mut params)?;
    Ok(Prepared {
        auth_headers,
        api_base,
        params,
        options,
    })
}
