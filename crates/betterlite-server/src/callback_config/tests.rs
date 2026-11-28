#![cfg(test)]

mod alerts;
mod delete;
mod get;
mod support;
mod yaml;

use crate::callback_config::ports::{CallbackClock as _, SystemCallbackClock};

// deleted_at comes from the proxy's own wall clock, which the harness replaces with a fixed instant
#[test]
fn system_callback_clock_reads_the_local_wall_clock() {
    let before = chrono::Local::now().naive_local();
    let observed = SystemCallbackClock.now().naive_local();
    let after = chrono::Local::now().naive_local();
    assert!(observed >= before);
    assert!(observed <= after);
}
