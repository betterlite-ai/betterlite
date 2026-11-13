use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_provider::api::responses_provider_contract::records::Params;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

mod fixture;
mod ops;

#[derive(Deserialize)]
pub(crate) struct Sdk {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct UrlProbe {
    pub(super) api_base: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateProbe {
    pub(super) model: String,
    pub(super) input: Json,
    #[serde(default)]
    pub(super) optional_params: Params,
}

#[derive(Deserialize)]
pub(crate) struct ReplyProbe {
    pub(super) body: String,
    pub(super) status: u16,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct GetProbe {
    pub(super) response_id: String,
    pub(super) api_base: String,
}

#[derive(Deserialize)]
pub(crate) struct Urls {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) sdk: Sdk,
    pub(super) probes: Vec<UrlProbe>,
}

#[derive(Deserialize)]
pub(crate) struct Creates {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) sdk: Sdk,
    pub(super) probes: Vec<CreateProbe>,
}

#[derive(Deserialize)]
pub(crate) struct Replies {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) sdk: Sdk,
    pub(super) now: i64,
    pub(super) probes: Vec<ReplyProbe>,
}

#[derive(Deserialize)]
pub(crate) struct Gets {
    pub(super) probes: Vec<GetProbe>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Urls(Urls),
    CreateRequests(Creates),
    Replies(Replies),
    GetRequests(Gets),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Urls(call) => Ok(ops::urls(call)),
            Input::CreateRequests(call) => Ok(ops::creates(call)),
            Input::Replies(call) => Ok(ops::replies(call)),
            Input::GetRequests(call) => Ok(ops::gets(call)),
        }
    }
}
