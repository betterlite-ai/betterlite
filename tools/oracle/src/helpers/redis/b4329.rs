mod fake_server;
mod input;
mod operations;
mod projection;

use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;

use serde_json::json;
use tokio::runtime::Builder;

use self::fake_server::FakeServer;
use self::input::{Call, ListValue};
use self::operations::{RecordingHooks, build, run};
use self::projection::{events, lists};
use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| helper_error("RuntimeError", error))?;
        runtime.block_on(run_case(input))
    }
}

async fn run_case(call: &Call) -> HelperResult {
    let initial: BTreeMap<String, Vec<Vec<u8>>> = call
        .lists
        .iter()
        .map(|(key, values)| (key.clone(), values.iter().map(ListValue::stored).collect()))
        .collect();
    let server = FakeServer::start(initial)
        .await
        .map_err(|error| helper_error("RedisError", error))?;
    let hooks = Arc::new(RecordingHooks::default());
    let operations = build(&server, call, Arc::clone(&hooks))?;
    let result = run(call, &operations).await?;
    Ok(json!({
        "result": result,
        "lists": lists(&server.lists()),
        "events": events(&hooks.events()),
    }))
}

pub(crate) fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
