use std::collections::BTreeMap;

use betterlite_azure::credentials::source::SourceFailure;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

mod fixture;
mod steps;
mod tests;

use fixture::Script;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Token(Vec<TokenStep>),
    RequestHeaders(Vec<HeaderStep>),
    RedactHeaders(Vec<HeaderStep>),
    ResponseHeaders(Vec<ResponseStep>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenStep {
    cache: Vec<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    params: BTreeMap<String, String>,
    #[serde(default)]
    refresh: bool,
    provider: Option<Script>,
    #[serde(default)]
    sources: BTreeMap<String, Script>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderStep {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    client: ClientParams,
    provider: Option<Script>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientParams {
    azure_ad_token: Option<String>,
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
