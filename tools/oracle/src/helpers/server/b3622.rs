use std::collections::BTreeMap;

use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::vertex_passthrough_auth::base_url::passthrough_vertex_base_url;
use betterlite_server::vertex_passthrough_auth::headers::vertex_upstream_headers;
use betterlite_server::vertex_passthrough_auth::override_params::{
    RouterCredentials, override_vertex_params,
};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    BaseUrl {
        location: Option<String>,
    },
    OverrideParams {
        router_credentials: Option<InputRouter>,
        project: Option<String>,
        location: Option<String>,
    },
    UpstreamHeaders {
        endpoint: String,
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputRouter {
    litellm_params: Option<JsonObject>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::BaseUrl { location } => {
                let url = passthrough_vertex_base_url(location.as_deref()).or_value_error()?;
                Ok(json!(url))
            }
            Input::OverrideParams {
                router_credentials,
                project,
                location,
            } => Ok(json!(override_answer(
                router_credentials.as_ref(),
                project,
                location,
            ))),
            Input::UpstreamHeaders { endpoint, headers } => {
                Ok(json!(headers_answer(endpoint, headers)))
            }
        }
    }
}

fn override_answer(
    router: Option<&InputRouter>,
    project: &Option<String>,
    location: &Option<String>,
) -> (Option<String>, Option<String>) {
    let record = router.map(|found| RouterCredentials {
        litellm_params: found.litellm_params.clone(),
    });
    override_vertex_params(record.as_ref(), project.clone(), location.clone())
}

fn headers_answer(endpoint: &str, headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let incoming: IndexMap<String, SecretString> = headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect();
    vertex_upstream_headers(endpoint, &incoming)
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}
