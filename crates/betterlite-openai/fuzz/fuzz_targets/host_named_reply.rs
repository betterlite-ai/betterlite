#![no_main]

use betterlite_openai::compatible_wire::{NEW_ID, system_clock};
use betterlite_openai::host_model_identity::{
    HostReplyMode, ReplyDefaults, decode_host_named_reply,
};
use libfuzzer_sys::fuzz_target;

// an even tag is Clarifai, an odd one CompactifAI, and its second bit turns on json mode
const fn mode(tag: u8) -> HostReplyMode<'static> {
    if tag.is_multiple_of(2) {
        HostReplyMode::Clarifai {
            request_model: "fuzz-model",
        }
    } else {
        HostReplyMode::Compactifai {
            request_model: "fuzz-model",
            json_mode: tag & 2 != 0,
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((tag, body)) = data.split_first() else {
        return;
    };
    let defaults = ReplyDefaults {
        identifier: NEW_ID,
        clock: system_clock,
    };
    let _decoded = decode_host_named_reply(body, mode(*tag), defaults);
});
