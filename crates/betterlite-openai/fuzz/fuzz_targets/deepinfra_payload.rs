#![no_main]

use betterlite_openai::deepinfra::rerank::RerankLimits;
use betterlite_openai::deepinfra::rerank::response::{
    decode_deepinfra_error, decode_deepinfra_rerank,
};
use libfuzzer_sys::fuzz_target;

// the first byte tags the entry point: an even byte feeds the rerank body decoder, an odd one the error decoder
fuzz_target!(|bytes: &[u8]| {
    let limits = RerankLimits {
        max_body_bytes: 0x0010_0000,
    };
    if let Some((&tag, payload)) = bytes.split_first() {
        if tag % 2 == 0 {
            let _decoded = decode_deepinfra_rerank(payload, limits);
        } else if let Ok(message) = std::str::from_utf8(payload) {
            let _extracted = decode_deepinfra_error(message, limits);
        }
    }
});
