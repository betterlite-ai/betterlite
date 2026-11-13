use std::collections::BTreeMap;

use betterlite_config::sso_pkce::load_pkce_settings;
use betterlite_server::sso_pkce::{OsEntropy, PkceError, generic_sso_redirect_params};
use serde::Deserialize;

use crate::helpers::HelperError;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

fn failure(error: PkceError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "PkceError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let settings = load_pkce_settings(&input.environment);
        let redirect = generic_sso_redirect_params(input.state.as_deref(), &settings, &OsEntropy)
            .map_err(failure)?;
        serde_json::to_value(&redirect.params).map_err(|error| {
            Box::new(HelperError {
                class: "JsonError".to_owned(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: error.to_string(),
            })
        })
    }
}
