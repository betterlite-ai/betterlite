#![no_main]

use betterlite_core::api::transcription_protocol::events::UuidEventIds;
use betterlite_core::api::transcription_protocol::json_object;
use betterlite_core::meta_muse_realtime::MuseMode;
use betterlite_core::meta_muse_realtime::MuseProtocolError;
use betterlite_core::meta_muse_realtime::events::MuseEventTransformer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Some((&selector, rest)) = bytes.split_first() else {
        return;
    };
    let mode = match selector {
        0 => MuseMode::Endpointing,
        1 => MuseMode::PushToTalk,
        _ => return,
    };
    let Ok(frame) = json_object::<MuseProtocolError>(rest) else {
        return;
    };
    let mut transformer = MuseEventTransformer::default();
    let mut ids = UuidEventIds;
    drop(transformer.transform(&frame, mode, &mut ids));
    drop(transformer.take_unbilled_usage());
});
