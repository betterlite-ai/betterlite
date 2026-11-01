use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_defaults_b::model::ProviderDefaultsRequest;
use betterlite_models::provider_defaults_b::resolve::resolve_provider_defaults;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    provider: ProviderId,
    model: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    global_api_key: Option<String>,
    #[serde(default)]
    global_api_base: Option<String>,
    #[serde(default)]
    xai_key: Option<String>,
    #[serde(default)]
    inception_key: Option<String>,
    #[serde(default)]
    lemonade_key: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let request = ProviderDefaultsRequest {
            provider: input.provider,
            model: input.model.clone(),
            api_base: input.api_base.clone(),
            api_key: input.api_key.clone().map(SecretString::from),
        };
        let globals = GlobalCredentials {
            api_key: input.global_api_key.clone().map(SecretString::from),
            api_base: input.global_api_base.clone(),
            xai_key: input.xai_key.clone(),
            inception_key: input.inception_key.clone(),
            lemonade_key: input.lemonade_key.clone(),
        };
        let resolved =
            resolve_provider_defaults(request, &globals, &input.environment).map_err(|error| {
                Box::new(HelperError {
                    class: "UnknownProvider".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        Ok(json!({
            "model": resolved.model,
            "api_base": resolved.api_base,
            "api_key": resolved.api_key.as_ref().map(secrecy::ExposeSecret::expose_secret),
        }))
    }
}
