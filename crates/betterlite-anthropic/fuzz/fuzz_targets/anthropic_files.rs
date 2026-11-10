#![no_main]
use betterlite_anthropic::files::wire::decode_anthropic_file;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|input: &[u8]| {
    let _decoded = decode_anthropic_file(input, 1_700_000_000);
});
