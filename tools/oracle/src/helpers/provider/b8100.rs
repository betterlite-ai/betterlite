use serde::Deserialize;

use self::script::{AgentCase, ProbeCase, RequestCase, ResponseCase, StreamCase};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod ops;
mod script;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Registry {
        agents: Vec<AgentCase>,
        probes: Vec<ProbeCase>,
    },
    Request {
        cases: Vec<RequestCase>,
    },
    RequestEmpty,
    Response {
        cases: Vec<ResponseCase>,
    },
    Stream {
        chunks: Vec<StreamCase>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Registry { agents, probes } => Ok(ops::registry(agents, probes)),
            Input::Request { cases } => ops::requests(cases),
            Input::RequestEmpty => Ok(ops::empty_request()),
            Input::Response { cases } => ops::responses(cases),
            Input::Stream { chunks } => ops::chunks(chunks),
        }
    }
}
