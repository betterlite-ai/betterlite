#![no_main]

use betterlite_server::pass_through_logging::vertex_batch_logging::testing::handle_batch_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(handle_batch_response(body));
});
