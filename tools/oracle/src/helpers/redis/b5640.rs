mod batch_read;
mod input;
mod ttl_reply;

use std::fmt::Display;

use self::input::Input;
use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::BatchRead(call) => batch_read::run(call),
            Input::TtlReply(call) => Ok(ttl_reply::run(call)),
        }
    }
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
