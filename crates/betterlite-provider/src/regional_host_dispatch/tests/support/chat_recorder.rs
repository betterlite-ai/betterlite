use std::sync::{Mutex, PoisonError};

use crate::hosted_platform_dispatch::call::CompletionCall;
use crate::hosted_platform_dispatch::handler::{ChatCompletionHandler, HandlerError};

pub(crate) struct ChatRecorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl ChatRecorder {
    pub(crate) const fn succeeding() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: None,
        }
    }

    pub(crate) const fn failing(status_code: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(status_code),
        }
    }

    pub(crate) fn first(&self) -> CompletionCall {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .expect("the arm called the handler")
    }
}

impl ChatCompletionHandler for ChatRecorder {
    type Reply = String;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        self.failure.map_or_else(
            || Ok(String::from("reply")),
            |status_code| Err(HandlerError::Upstream { status_code }),
        )
    }
}
