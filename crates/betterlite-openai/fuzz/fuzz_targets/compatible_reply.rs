#![no_main]
use betterlite_openai::compatible_models::decode_compatible_model_list;
use betterlite_openai::compatible_wire::{
    NEW_ID, ReplyOptions, decode_compatible_chat_reply, system_clock,
};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let Some((tag, body)) = data.split_first() else {
        return;
    };
    if tag.is_multiple_of(2) {
        let options = ReplyOptions {
            json_mode: tag.is_multiple_of(4),
            provider: Some("fuzz"),
            base_model: Some("base"),
            identifier: NEW_ID,
            clock: system_clock,
        };
        let _chat = decode_compatible_chat_reply(body, &options);
    } else {
        let _models = decode_compatible_model_list(body);
    }
});
