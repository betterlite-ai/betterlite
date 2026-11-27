#![no_main]

use betterlite_server::sso_callback_input::query::decode_oauth_callback_query;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| drop(decode_oauth_callback_query(bytes)));
