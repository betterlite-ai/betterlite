#![no_main]

use betterlite_secret_managers::vault::response::login::decode_login_response;
use betterlite_secret_managers::vault::response::{json_object_body, secret_value};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _login = decode_login_response(bytes);
    let _object = json_object_body(bytes);
    let _value = secret_value(bytes, "key");
});
