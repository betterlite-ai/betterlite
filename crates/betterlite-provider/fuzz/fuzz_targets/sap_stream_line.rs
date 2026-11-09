#![no_main]

use std::str;

use betterlite_provider::sap_stream::{Mode, SapLineStream};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(line) = str::from_utf8(bytes) else {
        return;
    };
    for mode in [Mode::Sync, Mode::Async] {
        let mut stream = SapLineStream {
            now: 1_700_000_000,
            done: false,
        };
        let _outcome = stream.next_chunk(line, mode);
    }
});
