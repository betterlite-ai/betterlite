#![no_main]

use betterlite_openai::voyage_rerank::{RerankLogging, decode_voyage_rerank};
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

struct Recorder;

impl RerankLogging for Recorder {
    fn post_call(&mut self, _original_response: &[u8]) {}
}

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_voyage_rerank(bytes, 200, Uuid::nil(), &mut Recorder);
});
