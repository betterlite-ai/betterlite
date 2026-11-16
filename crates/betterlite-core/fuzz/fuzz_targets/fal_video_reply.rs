#![no_main]

use betterlite_core::api::fal::video_prepare::VideoHeaders;
use betterlite_core::api::fal::video_prepare::VideoParams;
use betterlite_core::api::fal::video_prepare::create::{CreateResponseInput, create_response};
use betterlite_core::api::fal::video_prepare::decode::decode_fal_video_reply;
use betterlite_core::api::fal::video_prepare::urls::job_urls;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let headers = VideoHeaders::new();
    let Ok(reply) = decode_fal_video_reply(200, &headers, data) else {
        return;
    };
    let request_data = VideoParams::new();
    let Ok(video) = create_response(&CreateResponseInput {
        model: "workflows/fal-ai/name",
        reply: &reply,
        custom_llm_provider: None,
        request_data: &request_data,
        created_at: 0,
    }) else {
        return;
    };
    drop(job_urls(&video.id, "https://queue.fal.run"));
});
