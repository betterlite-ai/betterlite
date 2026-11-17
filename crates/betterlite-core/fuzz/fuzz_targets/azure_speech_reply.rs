#![no_main]

use betterlite_core::api::passthrough_logging::azure_speech::duration::billed_audio_seconds;
use betterlite_core::api::passthrough_logging::azure_speech::route::SpeechRoute;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(billed_audio_seconds(SpeechRoute::ShortAudio, None, data));
    drop(billed_audio_seconds(
        SpeechRoute::FastTranscription,
        None,
        data,
    ));
});
