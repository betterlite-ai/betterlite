#![no_main]

use std::str::from_utf8;

use betterlite_core::api::deepgram::listen_metadata::pricing::{
    addon_pricing_models, pricing_model,
};
use betterlite_core::api::deepgram::listen_metadata::query::decode_listen_query;
use betterlite_core::api::deepgram::listen_metadata::target::{
    parse_listen_target, websocket_target,
};
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 0x0004_0000;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MAX_INPUT {
        return;
    }
    let query = decode_listen_query(bytes);
    let _model: &str = query.requested_model();
    drop(query.first_occurrences());
    drop(query.callback_params());
    let _channels: Option<u64> = query.declared_channels();
    drop(websocket_target(None, &query));
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let Ok(target) = parse_listen_target(text) else {
        return;
    };
    drop(addon_pricing_models(&target));
    drop(pricing_model(&target));
});
