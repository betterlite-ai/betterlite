#![cfg(test)]

use rstest::rstest;

use super::{StackFrame, project_call_stack};

fn frame(function_name: &str, module_name: &str) -> StackFrame {
    StackFrame {
        function_name: function_name.to_owned(),
        module_name: module_name.to_owned(),
    }
}

#[test]
fn reports_caller_and_grandcaller() {
    let frames = [
        frame("get_cache", "redis_cache"),
        frame("handle_request", "proxy"),
    ];
    assert_eq!(
        project_call_stack(&frames, "redis_cache", 2),
        "get_cache <- handle_request"
    );
}

#[rstest]
#[case::lambda("<lambda>")]
#[case::wrapper("wrapper")]
#[case::sync_guard("_run_under_circuit_breaker_sync")]
#[case::async_guard("_run_under_circuit_breaker")]
fn skips_guard_frames_from_this_module(#[case] guard_name: &str) {
    let frames = [
        frame(guard_name, "redis_cache"),
        frame("get_cache", "redis_cache"),
    ];
    assert_eq!(project_call_stack(&frames, "redis_cache", 2), "get_cache");
}

#[test]
fn keeps_a_same_named_frame_from_another_module() {
    let frames = [
        frame("wrapper", "some_other_module"),
        frame("get_cache", "redis_cache"),
    ];
    assert_eq!(
        project_call_stack(&frames, "redis_cache", 2),
        "wrapper <- get_cache"
    );
}

#[test]
fn stops_at_num_frames() {
    let frames = [
        frame("one", "redis_cache"),
        frame("two", "redis_cache"),
        frame("three", "redis_cache"),
    ];
    assert_eq!(project_call_stack(&frames, "redis_cache", 2), "one <- two");
}

#[test]
fn falls_back_to_unknown_without_frames() {
    assert_eq!(project_call_stack(&[], "redis_cache", 2), "unknown");
}

#[test]
fn falls_back_to_unknown_when_every_frame_is_a_guard() {
    let frames = [frame("wrapper", "redis_cache")];
    assert_eq!(project_call_stack(&frames, "redis_cache", 2), "unknown");
}
