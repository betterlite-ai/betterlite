#![no_main]

use betterlite_core::api::realtime_ephemeral_ids::token::{
    TokenDecodeLimits, decode_realtime_token_payload,
};
use libfuzzer_sys::fuzz_target;

const LIMITS: TokenDecodeLimits = TokenDecodeLimits { max_bytes: 1 << 20 };

fuzz_target!(|bytes: &[u8]| {
    drop(decode_realtime_token_payload(bytes, LIMITS));
});
