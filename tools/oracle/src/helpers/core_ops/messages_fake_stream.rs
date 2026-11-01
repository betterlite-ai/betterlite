use betterlite_core::api::messages_fake_stream::value::Json;
use betterlite_core::api::messages_fake_stream::{MessagesFakeStream, MessagesFakeStreamError};
use serde_json::json;

use crate::helpers::HelperError;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Input {
    response: Json,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let stream = MessagesFakeStream::new(&input.response).map_err(fake_stream_error)?;
        let frames: Vec<String> = stream
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .collect();
        Ok(json!(frames))
    }
}

fn fake_stream_error(error: MessagesFakeStreamError) -> Box<HelperError> {
    let class = match error {
        MessagesFakeStreamError::ContentNotIterable => "TypeError",
        MessagesFakeStreamError::ContentBlockNotMapping => "AttributeError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
