mod cache;
mod selection;
mod state;
mod tests;
mod watch;

use std::sync::Arc;

use betterlite_core::api::responses_poll_state::DEFAULT_TTL;
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
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default = "default_now")]
    now: i64,
    #[serde(default = "default_ttl")]
    ttl: i64,
    #[serde(default = "default_uuid")]
    uuid: String,
    #[serde(default)]
    calls: Vec<StateCall>,
    #[serde(default)]
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
            let calls = state::calls(input, uuid, &cache, &lines).await?;
            let selections = selection::selections(input, &lines)?;
            Ok::<Value, Box<HelperError>>(json!({
                "calls": calls,
                "selections": selections,
            }))
        });
        drop(guard);
        output
    }
}

const fn default_now() -> i64 {
    1_700_000_000
}

const fn default_ttl() -> i64 {
    DEFAULT_TTL
}

fn default_uuid() -> String {
    "00000000-0000-4000-8000-000000000001".to_owned()
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
