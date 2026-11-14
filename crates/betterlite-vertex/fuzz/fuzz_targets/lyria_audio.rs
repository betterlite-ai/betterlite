#![no_main]

use betterlite_models::metadata::vendor::VertexAudioApi;
use betterlite_vertex::vertex_lyria::audio::{LyriaDecode, decode_lyria_dialect};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let decode = LyriaDecode {
        model: "lyria-002",
        status: 200,
        body: bytes,
    };
    for api in [
        VertexAudioApi::LyriaPredict,
        VertexAudioApi::LyriaInteractions,
    ] {
        drop(decode_lyria_dialect(api, &decode, &|_audio| None));
    }
});
