#![no_main]

use betterlite_provider::nvidia_nim_rerank::reply::decode_nvidia_nim_rerank_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_nvidia_nim_rerank_reply(bytes);
});
