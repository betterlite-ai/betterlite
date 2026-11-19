#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger as _, StreamPostCall};
use betterlite_logging::logging_pre_post::env::{LogLevel, LogRecord, LogSink as _};
use indexmap::IndexMap;
use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::anthropic_dispatch::gateway::logging::{TracingSink, TracingStreamLogger};

#[derive(Clone, Default)]
struct DebugEvents(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for DebugEvents {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::DEBUG {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn emitted_events(run: impl FnOnce()) -> usize {
    let events = DebugEvents::default();
    let subscriber = tracing_subscriber::registry().with(events.clone());
    with_default(subscriber, run);
    events.0.load(Ordering::Relaxed)
}

#[test]
fn the_production_sink_writes_the_record_as_a_debug_event() {
    let events = emitted_events(|| {
        TracingSink.emit(LogRecord {
            level: LogLevel::Debug,
            message: String::from("record"),
            fields: IndexMap::new(),
        });
    });
    assert_eq!(events, 1);
}

#[test]
fn the_stream_logger_writes_the_post_call_as_a_debug_event() {
    let input = betterlite_models::decisions::Json::Null;
    let events = emitted_events(|| {
        TracingStreamLogger.post_call(StreamPostCall {
            input: &input,
            original_response: "answer",
            complete_input_dict: "{}",
        });
    });
    assert_eq!(events, 1);
}
