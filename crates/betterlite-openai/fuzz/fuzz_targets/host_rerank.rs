#![no_main]

use betterlite_openai::host_rerank::{HostProvider, decode_host_rerank};
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

fuzz_target!(|bytes: &[u8]| {
    for provider in [HostProvider::Jina, HostProvider::Infinity] {
        let _reply = decode_host_rerank(provider, 200, bytes, Uuid::nil, |_| {});
    }
});
