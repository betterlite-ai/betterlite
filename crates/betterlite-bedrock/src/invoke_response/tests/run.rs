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
        match &self.reply {
            Ok(InvokeReply::Chat(response)) => response,
            Ok(InvokeReply::Anthropic(_) | InvokeReply::Nova(_) | InvokeReply::Pegasus(_)) => {
                panic!("the reply is not a base family chat response")
            }
            Err(_error) => panic!("the reply failed"),
        }
    }

    pub(super) fn envelope(&self) -> &ResponseEnvelope {
        match &self.reply {
            Ok(InvokeReply::Anthropic(envelope)) => envelope,
            Ok(InvokeReply::Chat(_) | InvokeReply::Nova(_) | InvokeReply::Pegasus(_)) => {
                panic!("the reply is not an Anthropic envelope")
            }
            Err(_error) => panic!("the reply failed"),
        }
    }

    pub(super) fn nova(&self) -> &ModelResponse {
        match &self.reply {
            Ok(InvokeReply::Nova(response)) => response,
            Ok(InvokeReply::Chat(_) | InvokeReply::Anthropic(_) | InvokeReply::Pegasus(_)) => {
                panic!("the reply is not a Nova response")
            }
            Err(_error) => panic!("the reply failed"),
        }
    }

    pub(super) fn pegasus(&self) -> &PegasusResponse {
        match &self.reply {
            Ok(InvokeReply::Pegasus(response)) => response,
            Ok(InvokeReply::Chat(_) | InvokeReply::Anthropic(_) | InvokeReply::Nova(_)) => {
                panic!("the reply is not a Pegasus response")
            }
            Err(_error) => panic!("the reply failed"),
        }
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
