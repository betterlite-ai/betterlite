#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::transcription_protocol::audio::decode_pcm16_append;
use betterlite_core::api::transcription_protocol::{
    // ast-grep-ignore: no-long-names
    RealtimeTranscriptionProtocolError as ProtocolError,
    // ast-grep-ignore: no-long-names
    parse_transcription_session_update as parse_update,
};
use betterlite_models::decisions::Json;
use libfuzzer_sys::fuzz_target;
use std::str::from_utf8;

const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 1 << 20,
    max_event_bytes: 1 << 20,
    max_items: 64,
};

fuzz_target!(|bytes: &[u8]| {
    let Some((&selector, rest)) = bytes.split_first() else {
        return;
    };
    match selector {
        0 => drop(parse_update::<ProtocolError>(rest, LIMITS)),
        1 => {
            let Ok(text) = from_utf8(rest) else {
                return;
            };
            let audio = Json::Text(text.to_owned());
            drop(decode_pcm16_append::<ProtocolError>(Some(&audio), None));
        }
        _ => {}
    }
});
