#![no_main]

use betterlite_server::guardrails::admin_mutations::body::{
    decode_patch_body, decode_write_request,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _write = decode_write_request(body);
    let _patch = decode_patch_body(body);
});
