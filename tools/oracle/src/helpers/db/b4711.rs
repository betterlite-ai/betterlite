use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

mod calls;
mod rows;
mod views;

use calls::{
    active_submissions, create_if_free, find_conflict, get_all, get_server, pinned_tools, review,
    submissions,
};

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    FindConflict {
        server_name: Option<String>,
        alias: Option<String>,
        exclude_server_id: Option<String>,
        rows: Vec<Value>,
    },
    CreateIfFree {
        server_name: Value,
        alias: Value,
        rows: Vec<Value>,
    },
    GetAll {
        salt_key: String,
        approval_status: Option<String>,
        server_ids: Option<Vec<String>>,
        rows: Vec<Value>,
    },
    GetServer {
        salt_key: String,
        server_id: String,
        rows: Vec<Value>,
    },
    Review {
        salt_key: String,
        action: ReviewAction,
        server_id: String,
        touched_by: String,
        review_notes: Option<String>,
        rows: Vec<Value>,
    },
    PinnedTools {
        salt_key: String,
        server_id: String,
        pinned_tools: Option<Value>,
        touched_by: String,
        rows: Vec<Value>,
    },
    Submissions {
        salt_key: String,
        rows: Vec<Value>,
    },
    ActiveSubmissions {
        user_id: String,
        rows: Vec<Value>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewAction {
    Approve,
    Reject,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|source| helper_error("RuntimeError", &source.to_string()))?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> Result<Value, Box<HelperError>> {
    match input {
        Input::FindConflict {
            server_name,
            alias,
            exclude_server_id,
            rows,
        } => {
            find_conflict(
                server_name.as_deref(),
                alias.as_deref(),
                exclude_server_id.as_deref(),
                rows,
            )
            .await
        }
        Input::CreateIfFree {
            server_name,
            alias,
            rows,
        } => create_if_free(server_name, alias, rows).await,
        Input::GetAll {
            salt_key,
            approval_status,
            server_ids,
            rows,
        } => {
            get_all(
                salt_key,
                approval_status.as_deref(),
                server_ids.as_deref(),
                rows,
            )
            .await
        }
        Input::GetServer {
            salt_key,
            server_id,
            rows,
        } => get_server(salt_key, server_id, rows).await,
        Input::Review {
            salt_key,
            action,
            server_id,
            touched_by,
            review_notes,
            rows,
        } => {
            review(
                salt_key,
                action,
                server_id,
                touched_by,
                review_notes.as_deref(),
                rows,
            )
            .await
        }
        Input::PinnedTools {
            salt_key,
            server_id,
            pinned_tools,
            touched_by,
            rows,
        } => pinned_tools(salt_key, server_id, pinned_tools.as_ref(), touched_by, rows).await,
        Input::Submissions { salt_key, rows } => submissions(salt_key, rows).await,
        Input::ActiveSubmissions { user_id, rows } => active_submissions(user_id, rows).await,
    }
}

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
