#![no_main]
use betterlite_server::model_deprecation::dates::decode_deprecation_date;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _date = decode_deprecation_date(text);
});
