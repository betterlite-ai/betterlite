#![no_main]
use betterlite_openai::compatible_models::decode_compatible_model_list;
use betterlite_openai::compatible_wire::{ReplyOptions, decode_compatible_chat_reply};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let Some((tag, body)) = data.split_first() else {
        return;
    };
    if tag % 2 == 0 {
        let options = ReplyOptions {
            json_mode: tag % 4 == 0,
            provider: Some("fuzz"),
            base_model: Some("base"),
        };
        let _chat = decode_compatible_chat_reply(body, &options);
    } else {
        let _models = decode_compatible_model_list(body);
    }
});
