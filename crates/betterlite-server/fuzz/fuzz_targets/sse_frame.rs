#![no_main]

use betterlite_server::stream_chunks::frames::pop_sse_frame;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(buffer) = str::from_utf8(data) else {
        return;
    };
    drop(pop_sse_frame(buffer));
});
