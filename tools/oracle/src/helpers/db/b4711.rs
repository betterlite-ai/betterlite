use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::OrHelperError as _;

mod calls;
mod rows;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    FindConflict(FindConflict),
    CreateIfFree(CreateIfFree),
    GetAll(GetAll),
    GetServer(GetServer),
    Review(Review),
    PinnedTools(PinnedTools),
    Submissions(Submissions),
    ActiveSubmissions(ActiveSubmissions),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindConflict {
    server_name: Option<String>,
    alias: Option<String>,
    exclude_server_id: Option<String>,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateIfFree {
    server_name: Option<String>,
    alias: Option<String>,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetAll {
    salt_key: String,
    approval_status: Option<String>,
    server_ids: Option<Vec<String>>,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetServer {
    salt_key: String,
    server_id: String,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewAction {
    Approve,
    Reject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Review {
    salt_key: String,
    action: ReviewAction,
    server_id: String,
    touched_by: String,
    review_notes: Option<String>,
    rows: Vec<JsonData>,
}

// PinnedMCPTool reads its schema through RawValue, which serde's tagged-enum buffer cannot carry
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PinnedTools {
    salt_key: String,
    server_id: String,
    pinned_tools: Option<Value>,
    touched_by: String,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Submissions {
    salt_key: String,
    rows: Vec<JsonData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActiveSubmissions {
    user_id: String,
    rows: Vec<JsonData>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .or_value_error()?;
        runtime.block_on(async {
            match input {
                Input::FindConflict(call) => calls::find_conflict(call).await,
                Input::CreateIfFree(call) => calls::create_if_free(call).await,
                Input::GetAll(call) => calls::get_all(call).await,
                Input::GetServer(call) => calls::get_server(call).await,
                Input::Review(call) => calls::review(call).await,
                Input::PinnedTools(call) => calls::pinned_tools(call).await,
                Input::Submissions(call) => calls::submissions(call).await,
                Input::ActiveSubmissions(call) => calls::active_submissions(call).await,
            }
        })
    }
}
