mod cache;
mod selection;
mod state;
mod tests;
mod watch;

use std::sync::Arc;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use uuid::Uuid;

use cache::ScriptedCache;
use selection::Selection;
use state::StateCall;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    now: i64,
    ttl: i64,
    uuid: String,
    calls: Vec<StateCall>,
    selections: Vec<Selection>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let uuid = Uuid::parse_str(&input.uuid).map_err(|source| invalid(source.to_string()))?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| invalid(source.to_string()))?;
        let cache = Arc::new(ScriptedCache::default());
        let (guard, lines) = watch::install();
        let output = runtime.block_on(async {
            let mut calls = Vec::with_capacity(input.calls.len());
            for call in &input.calls {
                calls.push(state::call_result(input, uuid, call, &cache, &lines).await?);
            }
            let mut selections = Vec::with_capacity(input.selections.len());
            for selection in &input.selections {
                selections.push(selection::result(selection, &lines)?);
            }
            Ok::<Value, Box<HelperError>>(json!({
                "calls": calls,
                "selections": selections,
            }))
        });
        drop(guard);
        output
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
