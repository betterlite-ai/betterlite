#![no_main]

use betterlite_bedrock::qwen_deepseek::reply::Reply;
use betterlite_bedrock::qwen_deepseek::{QwenProjection, decode_qwen_reply};
use libfuzzer_sys::fuzz_target;

const SEED: &str = r#"{"id":"chatcmpl-fuzz","object":"chat.completion","created":0,"model":"","choices":[{"index":0,"message":{"role":"assistant","content":null},"finish_reason":null}]}"#;

fn replay(bytes: &[u8]) {
    let Ok(seed) = serde_json::from_str::<Reply>(SEED) else {
        return;
    };
    for projection in [QwenProjection::Qwen2, QwenProjection::Qwen3] {
        let _outcome = decode_qwen_reply(projection, bytes, seed.clone());
    }
}

fuzz_target!(|bytes: &[u8]| {
    replay(bytes);
});
