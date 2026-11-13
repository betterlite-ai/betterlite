use std::collections::BTreeMap;

use betterlite_provider::api::file_contracts::config::FileParams;
use serde::Deserialize;

use crate::helpers::Helper;
use crate::helpers::HelperResult;

mod fixture;
mod ops;

#[derive(Deserialize)]
pub(crate) struct Sdk {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct FileSpec {
    pub(super) present: bool,
    pub(super) filename: Option<String>,
    pub(super) content: Vec<u8>,
}

#[derive(Deserialize)]
pub(crate) struct PlanCase {
    pub(super) file: FileSpec,
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) sdk: Sdk,
}

#[derive(Deserialize)]
pub(crate) struct ReplyCase {
    pub(super) litellm_params: FileParams,
    pub(super) body: String,
}

#[derive(Deserialize)]
pub(crate) struct HeaderCase {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) sdk: Sdk,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) api_key: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct UrlCase {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) sdk: Sdk,
    pub(super) api_base: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct HeadersAndUrl {
    pub(super) validation: Vec<HeaderCase>,
    pub(super) urls: Vec<UrlCase>,
}

#[derive(Deserialize)]
pub(crate) struct CreatePlan {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) now: i64,
    pub(super) cases: Vec<PlanCase>,
}

#[derive(Deserialize)]
pub(crate) struct CreateReply {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) now: i64,
    pub(super) cases: Vec<ReplyCase>,
}

#[derive(Deserialize)]
pub(crate) struct Requests {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) now: i64,
    pub(super) sdk: Sdk,
    pub(super) api_base: Option<String>,
    pub(super) file_ids: Vec<String>,
    pub(super) delete_bodies: Vec<String>,
    pub(super) purposes: Vec<Option<String>>,
    pub(super) body: String,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    HeadersAndUrl(HeadersAndUrl),
    CreatePlan(CreatePlan),
    CreateReply(CreateReply),
    Requests(Requests),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::HeadersAndUrl(call) => Ok(ops::headers_and_url(call)),
            Input::CreatePlan(call) => Ok(ops::create_plan(call)),
            Input::CreateReply(call) => Ok(ops::create_reply(call)),
            Input::Requests(call) => Ok(ops::requests(call)),
        }
    }
}
