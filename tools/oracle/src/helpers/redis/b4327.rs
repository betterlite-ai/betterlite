mod fake_server;
mod input;
mod values;

use serde_json::json;

use self::fake_server::{FakeServer, Replies};
use self::input::{Input, ReadCall, WriteCall};
use self::values::{build, run_read, run_write};
use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| helper_error("RuntimeError", error))?;
        runtime.block_on(dispatch(input))
    }
}

async fn dispatch(input: &Input) -> HelperResult {
    match input {
        Input::WriteCommands(call) => dispatch_write(call).await,
        Input::ReadValues(call) => dispatch_read(call),
    }
}

async fn dispatch_write(call: &WriteCall) -> HelperResult {
    let server = FakeServer::start(Replies::default())
        .map_err(|error| helper_error("ServerError", error))?;
    let values = build(
        &server,
        call.namespace.as_deref(),
        call.default_ttl,
        call.flush_size,
    )?;
    run_write(call, &values).await?;
    Ok(json!({
        "commands": recorded(&server),
        "buffered": values.buffered_count(),
    }))
}

fn dispatch_read(call: &ReadCall) -> HelperResult {
    let server =
        FakeServer::start(replies_of(call)).map_err(|error| helper_error("ServerError", error))?;
    let values = build(&server, call.namespace.as_deref(), None, None)?;
    run_read(call, &values, &server)
}

fn replies_of(call: &ReadCall) -> Replies {
    let mget: Vec<Option<Vec<u8>>> = call
        .replies
        .iter()
        .map(|reply| reply.as_ref().map(|text| text.clone().into_bytes()))
        .collect();
    let get = mget.first().cloned().flatten();
    Replies { get, mget }
}

fn recorded(server: &FakeServer) -> Vec<Vec<String>> {
    server
        .commands()
        .into_iter()
        .filter(|command| command.first().is_none_or(|name| name != "CLIENT"))
        .collect()
}

pub(crate) fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: std::fmt::Display,
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
