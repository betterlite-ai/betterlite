#![cfg(test)]

// the fixture B2316 already runs against the shared batch, reused here instead of copied
#[path = "../spend_counter_batch/tests/support.rs"]
pub(crate) mod support;

mod keys;
mod read;
mod request_batch;
mod scope;
mod state;
mod values;
