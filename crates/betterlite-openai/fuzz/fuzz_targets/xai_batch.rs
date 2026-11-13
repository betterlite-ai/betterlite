#![no_main]

use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::xai_batches::records::{
    XaiBatch, XaiBatchList, XaiBatchResultsPage, decode_xai_batch,
};
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
    let _decoded = match tag {
        1 => decode_xai_batch::<XaiBatchList>(body, &limits).map(drop),
        2 => decode_xai_batch::<XaiBatchResultsPage>(body, &limits).map(drop),
        _ => decode_xai_batch::<XaiBatch>(body, &limits).map(drop),
    };
});
