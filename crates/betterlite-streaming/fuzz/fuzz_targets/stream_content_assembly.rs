#![no_main]

use betterlite_streaming::stream_content_assembly::decode_stream_audio_fragment;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|fragment: String| {
    let _ = decode_stream_audio_fragment(&fragment, 8 * 1024 * 1024);
});
