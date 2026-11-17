#![no_main]
use betterlite_server::pass_through_logging::tinyfish::sse::run_id_from_sse_frames;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(run_id_from_sse_frames(bytes));
});
