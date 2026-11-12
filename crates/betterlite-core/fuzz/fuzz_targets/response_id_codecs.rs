#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::response_id_codecs::{ResponseIdCodecKind, decode_response_id_codecs};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((selector, id)) = data.split_first() else {
        return;
    };
    let kind = ResponseIdCodecKind::from_selector(*selector);
    let _decoded = decode_response_id_codecs(kind, id, ApiLimits::default());
});
