#![no_main]

use betterlite_vertex::chirp_backend::commands::ChirpCommand;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = ChirpCommand::try_from(bytes);
});
