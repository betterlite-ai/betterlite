#![cfg(test)]

#[path = "tests/support.rs"]
mod support;

#[path = "tests/vigil_guard_analysis.rs"]
mod vigil_guard_analysis;
#[path = "tests/vigil_guard_block_reason.rs"]
mod vigil_guard_block_reason;
#[path = "tests/vigil_guard_call_id.rs"]
mod vigil_guard_call_id;
#[path = "tests/vigil_guard_clamp.rs"]
mod vigil_guard_clamp;
#[path = "tests/vigil_guard_config.rs"]
mod vigil_guard_config;
#[path = "tests/vigil_guard_failure.rs"]
mod vigil_guard_failure;
#[path = "tests/vigil_guard_handler_protocol.rs"]
mod vigil_guard_handler_protocol;
#[path = "tests/vigil_guard_metadata.rs"]
mod vigil_guard_metadata;
#[path = "tests/vigil_guard_output.rs"]
mod vigil_guard_output;
#[path = "tests/vigil_guard_retry.rs"]
mod vigil_guard_retry;
#[path = "tests/vigil_guard_sanitize.rs"]
mod vigil_guard_sanitize;
#[path = "tests/vigil_guard_tool_extraction.rs"]
mod vigil_guard_tool_extraction;
#[path = "tests/vigil_guard_tool_update.rs"]
mod vigil_guard_tool_update;
#[path = "tests/vigil_guard_transient.rs"]
mod vigil_guard_transient;
