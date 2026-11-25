#![cfg(test)]

use super::QueueHooks;
use crate::sse_data_generator::types::StreamHooks as _;

#[test]
fn queue_hooks_ask_for_no_iterator_wrap_and_no_chunk_hook() {
    let hooks = QueueHooks { failure: None };
    assert!(!hooks.needs_iterator_wrap());
    assert!(!hooks.needs_per_chunk_hook());
}
