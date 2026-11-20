use std::collections::BTreeMap;

use betterlite_config::sso_pkce::load_pkce_settings;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_server::generic_sso_callback::failure::{
    GenericSsoErrorResponse, classify_generic_sso_error,
};
use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    message: String,
    source: Source,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    status: u16,
    error_type: ProxyErrorTypes,
    param: Option<String>,
    code: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = load_pkce_settings(&input.environment);
        let source = GenericSsoErrorResponse {
            status: input.source.status,
            error_type: input.source.error_type,
            param: input.source.param.clone(),
            code: input.source.code.clone(),
        };
        let classified = classify_generic_sso_error(&input.message, source, settings.use_pkce);
        Ok(serde_json::to_value(classified)?)
    }
}
