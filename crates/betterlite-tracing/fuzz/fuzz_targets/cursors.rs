#![no_main]

use betterlite_tracing::cursor::{decode_error_cursor, decode_trace_cursor};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: String| {
    drop(decode_trace_cursor(Some(&input)));
    drop(decode_error_cursor(Some(&input)));
});
