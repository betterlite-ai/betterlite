use std::collections::VecDeque;
use std::future::pending;
use std::io;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use betterlite_core::api::realtime_session_state::logging::{
    RealtimeLogging, RealtimeSuccessQueue, SessionInput, SuccessWork,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{Block, Run};

struct ClientState {
    frames: Mutex<VecDeque<String>>,
    block: Block,
    send_failure: Option<usize>,
    sent: Mutex<Vec<String>>,
    close_code: Mutex<Option<u16>>,
}

pub(super) struct FakeWebSocket {
    state: ClientState,
}

impl FakeWebSocket {
    pub(super) fn new(run: &Run) -> Self {
        Self {
            state: ClientState {
                frames: Mutex::new(run.frames.iter().cloned().collect()),
                block: run.block,
                send_failure: run.client_send_failure,
                sent: Mutex::new(Vec::new()),
                close_code: Mutex::new(None),
            },
        }
    }

    pub(super) fn sent(&self) -> Vec<String> {
        self.state
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn close_code(&self) -> Option<u16> {
        *self
            .state
            .close_code
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl RealtimeClientWebSocket for FakeWebSocket {
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        let frame = self
            .state
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        if let Some(text) = frame {
            return Ok(text);
        }
        if self.state.block == Block::Client {
            return pending().await;
        }
        Err(ClientError {
            message: "the client is gone".to_owned(),
            source: None,
        })
    }

    async fn send_text(&self, data: String) -> Result<(), ClientError<io::Error>> {
        let index = {
            let mut sent = self
                .state
                .sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let index = sent.len();
            sent.push(data);
            index
        };
        if self.state.send_failure == Some(index) {
            return Err(ClientError {
                message: "the client send failed".to_owned(),
                source: None,
            });
        }
        Ok(())
    }

    async fn close(&self, code: u16, _reason: Option<String>) {
        let mut recorded = self
            .state
            .close_code
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if recorded.is_none() {
            *recorded = Some(code);
        }
    }
}

#[derive(Default)]
pub(super) struct FakeLogging {
    details: Mutex<Vec<(String, Json)>>,
}

impl FakeLogging {
    pub(super) fn success_logged(&self) -> bool {
        self.details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|(key, value)| {
                key == "realtime_session_success_logged" && matches!(value, Json::Bool(true))
            })
    }
}

impl RealtimeLogging for FakeLogging {
    fn pre_call(&self, _input: &SessionInput<'_>, _api_key: &str) {}

    fn record_model_call_detail(&self, key: &str, value: Json) {
        self.details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value));
    }

    fn dispatch_success_handlers(&self, _messages: &[IndexMap<String, Json>]) -> SuccessWork {
        Box::pin(async {})
    }
}

pub(super) struct AcceptingQueue;

impl RealtimeSuccessQueue for AcceptingQueue {
    fn enqueue(&self, _work: SuccessWork) -> bool {
        true
    }
}
