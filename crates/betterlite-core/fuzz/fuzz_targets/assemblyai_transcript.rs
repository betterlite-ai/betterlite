#![no_main]

use betterlite_core::relay::assemblyai::poll::decode_assembly_transcript;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_assembly_transcript(data));
});
