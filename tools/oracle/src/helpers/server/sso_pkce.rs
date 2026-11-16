use std::collections::BTreeMap;

use betterlite_config::sso_pkce::load_pkce_settings;
use betterlite_server::sso_pkce::{OsEntropy, generic_sso_redirect_params};
use serde::Deserialize;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = load_pkce_settings(&input.environment);
        let redirect = generic_sso_redirect_params(input.state.as_deref(), &settings, &OsEntropy)
            .or_value_error()?;
        Ok(serde_json::to_value(&redirect.params)?)
    }
}
