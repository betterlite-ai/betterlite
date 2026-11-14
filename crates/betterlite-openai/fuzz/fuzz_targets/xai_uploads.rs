#![no_main]

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::xai_files::{XaiFile, XaiFileDeleted, XaiFileList, decode_xai_upload_reply};
use betterlite_openai::xai_stt::decode_xai_stt_reply;
use http::header::HeaderMap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    };
    let Some((tag, body)) = bytes.split_first() else {
        return;
    };
    let reply = |status: u16| UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.to_vec(),
    };
    let _decoded: Option<()> = match tag {
        b'2' => decode_xai_upload_reply::<XaiFileList>(body, &limits)
            .map(drop)
            .ok(),
        b'3' => decode_xai_upload_reply::<XaiFileDeleted>(body, &limits)
            .map(drop)
            .ok(),
        b'4' => decode_xai_stt_reply(&reply(200), &limits).map(drop).ok(),
        b'5' => decode_xai_stt_reply(&reply(500), &limits).map(drop).ok(),
        _ => decode_xai_upload_reply::<XaiFile>(body, &limits)
            .map(drop)
            .ok(),
    };
});
