mod capabilities;
mod cases;
mod ops;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Url { cases } => Ok(json!(cases.iter().map(ops::url_of).collect::<Vec<_>>())),
            Input::Headers { cases } => ops::headers(cases),
            Input::Params { cases } => ops::params(cases),
            Input::Messages { cases } => Ok(json!(
                cases.iter().map(ops::messages_of).collect::<Vec<_>>()
            )),
            Input::Response { cases } => Ok(json!(
                cases.iter().map(ops::response_of).collect::<Vec<_>>()
            )),
            Input::Stream { cases } => {
                Ok(json!(cases.iter().map(ops::stream_of).collect::<Vec<_>>()))
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Url { cases: Vec<cases::UrlCase> },
    Headers { cases: Vec<cases::HeadersCase> },
    Params { cases: Vec<cases::ParamsCase> },
    Messages { cases: Vec<cases::MessagesCase> },
    Response { cases: Vec<cases::ResponseCase> },
    Stream { cases: Vec<cases::StreamCase> },
}
