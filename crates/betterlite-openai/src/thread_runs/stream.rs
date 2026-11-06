use std::sync::Arc;

use betterlite_http::proxy::outbound::OutboundRequest;

use crate::assistants::client::ClientKind;
use crate::assistants::transport::AssistantsTransport;
use crate::thread_runs::client::send;
use crate::thread_runs::decode::{decode_run_event, run_event_frames};
use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::records::RunEvent;

// B161 bridges a Python event handler onto this seam; the stream reports every decoded event here.
pub trait RunEventHandler: Send + Sync {
    fn on_event(&self, event: &RunEvent);
    fn on_end(&self);
}

// The lazy manager the run wrappers return: nothing reaches the provider until it is consumed.
pub struct RunStream<T> {
    pub kind: ClientKind,
    pub handler: Option<Arc<dyn RunEventHandler>>,
    pub transport: Arc<T>,
    pub request: OutboundRequest,
}

impl<T: AssistantsTransport> RunStream<T> {
    // A failed entry never reaches the SDK's on_end, but an error after entry does.
    pub async fn consume(self) -> Result<Vec<RunEvent>, ThreadRunsError> {
        let Self {
            transport,
            request,
            handler,
            ..
        } = self;
        let response = send(transport.as_ref(), request).await?;
        let outcome = events_from(handler.as_ref(), &response.body);
        if let Some(sink) = handler.as_ref() {
            sink.on_end();
        }
        outcome
    }
}

fn events_from(
    handler: Option<&Arc<dyn RunEventHandler>>,
    body: &[u8],
) -> Result<Vec<RunEvent>, ThreadRunsError> {
    let mut events = Vec::new();
    for frame in run_event_frames(body) {
        let Some(event) = decode_run_event(frame)? else {
            break;
        };
        if let Some(sink) = handler {
            sink.on_event(&event);
        }
        events.push(event);
    }
    Ok(events)
}
