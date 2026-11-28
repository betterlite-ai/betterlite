#![no_main]

use betterlite_server::callback_config::body::{callback_delete_request, config_yaml_request};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(callback_delete_request(body));
    drop(config_yaml_request(body));
});
