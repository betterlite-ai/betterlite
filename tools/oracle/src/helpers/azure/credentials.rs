use std::collections::BTreeMap;

use betterlite_azure::credentials::source::SourceFailure;
use secrecy::SecretString;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

mod fixture;
mod steps;
mod tests;

use fixture::{ProviderScript, SourceScript};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Token { steps: Vec<TokenStep> },
    RequestHeaders { steps: Vec<HeaderStep> },
    RedactHeaders { steps: Vec<HeaderStep> },
    ResponseHeaders { steps: Vec<ResponseStep> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenStep {
    cache: Vec<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    params: Params,
    #[serde(default)]
    refresh: bool,
    #[serde(default)]
    provider: Option<ProviderScript>,
    #[serde(default)]
    sources: BTreeMap<String, SourceScript>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Params {
    #[serde(default)]
    azure_ad_token: Option<SecretString>,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<SecretString>,
    #[serde(default)]
    azure_scope: Option<String>,
    #[serde(default)]
    azure_username: Option<String>,
    #[serde(default)]
    azure_password: Option<SecretString>,
}

// SecretString refuses to serialize itself, so the config view reads the plain values the loader expects
#[derive(serde::Serialize)]
struct ParamsView<'params> {
    #[serde(skip_serializing_if = "Option::is_none")]
    azure_ad_token: Option<&'params str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_id: Option<&'params str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<&'params str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<&'params str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azure_scope: Option<&'params str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azure_username: Option<&'params str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azure_password: Option<&'params str>,
}

impl Params {
    fn view(&self) -> ParamsView<'_> {
        ParamsView {
            azure_ad_token: self
                .azure_ad_token
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
            tenant_id: self.tenant_id.as_deref(),
            client_id: self.client_id.as_deref(),
            client_secret: self
                .client_secret
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
            azure_scope: self.azure_scope.as_deref(),
            azure_username: self.azure_username.as_deref(),
            azure_password: self
                .azure_password
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderStep {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    client: ClientParams,
    #[serde(default)]
    provider: Option<ProviderScript>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientParams {
    #[serde(default)]
    azure_ad_token: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseStep {
    source: ResponseSource,
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseSource {
    Dict,
    Httpx,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input)
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn source_error(failure: SourceFailure) -> Box<HelperError> {
    Box::new(HelperError {
        class: failure.class,
        status: failure.status,
        error_type: None,
        param: None,
        code: None,
        message: failure.message,
    })
}
