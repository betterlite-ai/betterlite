#![no_main]

use betterlite_logging::otlp_errors::CodecError;
use betterlite_logging::otlp_normalize::{DecodedSpan, SpanCodec, decode_otlp};
use libfuzzer_sys::fuzz_target;

struct JsonCodec;

impl SpanCodec for JsonCodec {
    fn decode(
        &self,
        payload: &[u8],
        _content_type: Option<&str>,
    ) -> Result<Vec<DecodedSpan>, CodecError> {
        serde_json::from_slice(payload).map_err(|error| CodecError::Malformed(error.to_string()))
    }
}

fuzz_target!(|body: &[u8]| {
    let _ = decode_otlp(&JsonCodec, body, Some("application/json"), None);
    let _ = decode_otlp(&JsonCodec, body, Some("application/json"), Some("gzip"));
});
