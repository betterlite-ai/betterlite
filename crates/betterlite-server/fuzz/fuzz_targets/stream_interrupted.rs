#![no_main]

use betterlite_server::pass_through_logging::interrupted_stream_usage::testing::stream_was_interrupted;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _decided = stream_was_interrupted(&[input.to_vec()]);
});
