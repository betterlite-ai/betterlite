#![no_main]

use betterlite_openai::hosted_vllm_media::rerank::reply::transform_rerank_response;
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

fuzz_target!(|bytes: &[u8]| {
    let _reply = transform_rerank_response(200, bytes, &|| Uuid::nil());
});
