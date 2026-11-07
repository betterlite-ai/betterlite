use betterlite_logging::batch::retry::{
    RequeuePolicy, is_retryable_status, undelivered_after_http_error,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod flush;
mod send;

use flush::{FlushScenario, flush_outcome};
use send::{SendScenario, send_outcome};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    statuses: Option<StatusScenario>,
    #[serde(default)]
    send: Option<SendScenario>,
    #[serde(default)]
    flush: Option<FlushScenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusScenario {
    events: Vec<String>,
    codes: Vec<u16>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(failure)?;
        let statuses = input
            .statuses
            .as_ref()
            .map_or(Value::Null, |scenario| Value::Array(status_rows(scenario)));
        let send = input.send.as_ref().map_or(Value::Null, |scenario| {
            runtime.block_on(send_outcome(scenario))
        });
        let flush = input.flush.as_ref().map_or(Value::Null, |scenario| {
            runtime.block_on(flush_outcome(scenario))
        });
        Ok(json!({"statuses": statuses, "send": send, "flush": flush}))
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "BatchDeliveryError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn status_rows(scenario: &StatusScenario) -> Vec<Value> {
    scenario
        .codes
        .iter()
        .map(|code| {
            let undelivered = undelivered_after_http_error(
                &scenario.events,
                *code,
                "BatchDelivery",
                "classification",
            );
            let requeued = RequeuePolicy::RequeueAll.retained(
                &scenario.events,
                *code,
                "BatchDelivery",
                "classification",
            );
            json!({
                "code": code,
                "retryable": is_retryable_status(*code),
                "undelivered": undelivered,
                "requeued": requeued,
            })
        })
        .collect()
}
