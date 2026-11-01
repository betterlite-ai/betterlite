#![no_main]

use betterlite_cli::pkce_login::wire::{
    error_detail, parse_contract, parse_registered_client, parse_token,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _token = parse_token(bytes);
    let _client = parse_registered_client(bytes);
    let _contract = parse_contract(bytes);
    let _detail = error_detail(bytes);
});
