#![cfg(test)]

use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_openai::chat::response::ChatResponse;

use crate::converse_response::wire::ModelResponse;
use crate::invoke_response::InvokeReply;
use crate::invoke_response::error::InvokeReplyError;
use crate::pegasus::response::PegasusResponse;

use super::counter::CounterCall;

pub(super) struct Run {
    pub(super) reply: Result<InvokeReply, InvokeReplyError>,
    pub(super) calls: Vec<CounterCall>,
}

impl Run {
    pub(super) fn chat(&self) -> &ChatResponse {
        let Ok(InvokeReply::Chat(response)) = &self.reply else {
            panic!("the reply is not a base family chat response");
        };
        response
    }

    pub(super) fn envelope(&self) -> &ResponseEnvelope {
        let Ok(InvokeReply::Anthropic(envelope)) = &self.reply else {
            panic!("the reply is not an Anthropic envelope");
        };
        envelope
    }

    pub(super) fn nova(&self) -> &ModelResponse {
        let Ok(InvokeReply::Nova(response)) = &self.reply else {
            panic!("the reply is not a Nova response");
        };
        response
    }

    pub(super) fn pegasus(&self) -> &PegasusResponse {
        let Ok(InvokeReply::Pegasus(response)) = &self.reply else {
            panic!("the reply is not a Pegasus response");
        };
        response
    }

    pub(super) fn failure(&self) -> &InvokeReplyError {
        match &self.reply {
            Ok(_reply) => panic!("the reply unexpectedly succeeded"),
            Err(error) => error,
        }
    }

    pub(super) fn status(&self) -> Option<u16> {
        self.failure()
            .bedrock()
            .map(|error| error.error.status_code)
    }

    pub(super) fn message(&self) -> &str {
        self.failure()
            .bedrock()
            .map_or("", |error| error.error.message.as_str())
    }

    pub(super) fn response_headers(&self) -> &[(String, String)] {
        self.failure()
            .bedrock()
            .map_or(&[] as &[(String, String)], |error| {
                error.response_headers.as_slice()
            })
    }
}
