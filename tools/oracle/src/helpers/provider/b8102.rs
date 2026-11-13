use serde::Deserialize;

use self::script::{HeadersCase, InfoCase, ReplyCase, RequestCase, UrlCase};
use crate::helpers::Helper;
use crate::helpers::HelperResult;

mod ops;
mod script;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Info { cases: Vec<InfoCase> },
    Url { cases: Vec<UrlCase> },
    Headers { cases: Vec<HeadersCase> },
    Request { cases: Vec<RequestCase> },
    Reply { cases: Vec<ReplyCase> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Info { cases } => Ok(ops::info(cases)),
            Input::Url { cases } => Ok(ops::urls(cases)),
            Input::Headers { cases } => Ok(ops::headers(cases)),
            Input::Request { cases } => ops::requests(cases),
            Input::Reply { cases } => ops::replies(cases),
        }
    }
}
