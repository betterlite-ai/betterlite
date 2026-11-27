#![no_main]
use betterlite_server::login_return::origin::validate_return_to;
use betterlite_server::login_return::return_destination::{
    decode_return_destination, is_same_origin_return_path,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(destination) = decode_return_destination(bytes) else {
        return;
    };
    drop(is_same_origin_return_path(&destination));
    drop(validate_return_to(&destination, Some("https://cp.example")));
});
