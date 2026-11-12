use serde::Deserialize;

use self::script::{FlowInput, ReplyCase, RequestCase, SignCase};
use crate::helpers::Helper;
use crate::helpers::HelperResult;

mod fixture;
mod ops;
mod script;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Flow(FlowInput),
    Request {
        requests: Vec<RequestCase>,
        signatures: Vec<SignCase>,
    },
    Reply {
        replies: Vec<ReplyCase>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Flow(flow) => ops::flow(flow),
            Input::Request {
                requests,
                signatures,
            } => Ok(ops::requests(requests, signatures)),
            Input::Reply { replies } => ops::replies(replies),
        }
    }
}
