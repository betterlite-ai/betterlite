use std::collections::BTreeMap;

use betterlite_openai::evals::types::{
    CreateEvalRequest, CreateRunRequest, ListEvalsParams, ListRunsParams, UpdateEvalRequest,
};
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperError, HelperResult};

mod calls;
mod requests;
mod tests;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Get,
    Delete,
    Cancel,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplyOperation {
    CreateEval,
    ListEvals,
    GetEval,
    UpdateEval,
    DeleteEval,
    CancelEval,
    CreateRun,
    ListRuns,
    GetRun,
    CancelRun,
    DeleteRun,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LitellmParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    pub(crate) operation: Option<Operation>,
    #[serde(default)]
    pub(crate) reply: Option<ReplyOperation>,
    #[serde(default)]
    pub(crate) eval_id: Option<String>,
    #[serde(default)]
    pub(crate) run_id: Option<String>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) endpoint: Option<String>,
    #[serde(default)]
    pub(crate) headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) litellm_params: Option<LitellmParams>,
    #[serde(default)]
    pub(crate) litellm_api_key: Option<String>,
    #[serde(default)]
    pub(crate) litellm_openai_key: Option<String>,
    #[serde(default)]
    pub(crate) env: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) create: Option<CreateEvalRequest>,
    #[serde(default)]
    pub(crate) update: Option<UpdateEvalRequest>,
    #[serde(default)]
    pub(crate) create_run: Option<CreateRunRequest>,
    #[serde(default)]
    pub(crate) list_params: Option<ListEvalsParams>,
    #[serde(default)]
    pub(crate) list_runs_params: Option<ListRunsParams>,
    #[serde(default)]
    pub(crate) status: Option<u16>,
    #[serde(default)]
    pub(crate) text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probes {
    pub(crate) probes: Vec<Probe>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    EvalCreateUpdate(Probes),
    EvalListPagination(Probes),
    EvalGetDeleteCancel(Probes),
    RunCreateDataSource(Probes),
    RunListPagination(Probes),
    RunGetCancelDelete(Probes),
    MalformedJsonError(Probes),
    EvalAuthRequired(Probes),
    EvalUrlLiteralAppend(Probes),
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
