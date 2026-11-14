#![no_main]

use betterlite_vertex::chirp_backend::commands::decode_chirp_command;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_chirp_command(bytes);
});
