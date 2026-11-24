#![cfg(test)]

use std::sync::Arc;

use super::PassThroughStreamContext;
use crate::passthrough_stream_lifecycle::support::{passthrough, time};

fn context() -> PassThroughStreamContext {
    PassThroughStreamContext {
        success_handler: passthrough(),
        url_route: "/v1/messages".to_owned(),
        start_time: time("2024-01-01T00:00:00+00:00"),
    }
}

#[test]
fn the_context_carries_the_failure_accounting_values() {
    let handler = passthrough();
    let context = PassThroughStreamContext {
        success_handler: Arc::clone(&handler),
        url_route: "/v1/messages".to_owned(),
        start_time: time("2024-01-01T00:00:00+00:00"),
    };
    assert!(Arc::ptr_eq(&context.success_handler, &handler));
    assert_eq!(context.url_route, "/v1/messages");
    assert_eq!(context.start_time, time("2024-01-01T00:00:00+00:00"));
}

#[test]
fn cloning_the_context_keeps_the_same_success_handler_and_values() {
    let original = context();
    let clone = original.clone();
    assert!(Arc::ptr_eq(
        &clone.success_handler,
        &original.success_handler
    ));
    assert_eq!(clone.url_route, original.url_route);
    assert_eq!(clone.start_time, original.start_time);
}

#[test]
fn separate_contexts_keep_separate_routes() {
    let mut other = context();
    other.url_route = "/v1/chat/completions".to_owned();
    assert_ne!(other.url_route, context().url_route);
}
