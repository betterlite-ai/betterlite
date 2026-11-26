#![no_main]
use betterlite_server::model_info_search::datetime::decode_model_info_datetime;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _decoded = decode_model_info_datetime(text);
});
