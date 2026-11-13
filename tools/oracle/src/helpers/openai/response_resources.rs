mod calls;
mod fixtures;
mod tests;

use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_resources::request::InputItemsOrder;
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Endpoint {
    Delete,
    Get,
    List,
    Cancel,
    Compact,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub(crate) endpoint: Option<Endpoint>,
    pub(crate) response_id: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) status: Option<u16>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) now: Option<i64>,
    pub(crate) model: Option<String>,
    pub(crate) input: Option<ResponsesInput>,
    pub(crate) params: Option<Object>,
    pub(crate) provider: Option<ProviderId>,
    pub(crate) model_info: Option<Object>,
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) include: Option<Vec<String>>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<InputItemsOrder>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probes {
    pub(crate) probes: Vec<Probe>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    EncodedResponseId(Probes),
    DeleteResult(Probes),
    GetStrictResponse(Probes),
    ListDefaultLimitOrder(Probes),
    IncludeCommaQuery(Probes),
    CancelResponse(Probes),
    CompactKeepsQuery(Probes),
    CompactReplayInput(Probes),
    CompactLaxResponse(Probes),
    ResourceJsonErrorStatus(Probes),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        calls::run(input)
    }
}

pub(super) fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
