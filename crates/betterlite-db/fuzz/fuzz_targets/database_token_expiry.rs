#![no_main]

use betterlite_db::database_tokens::expiry::{TokenAuthKind, decode_database_token_expiry};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _rds = decode_database_token_expiry(bytes, TokenAuthKind::Rds);
    let _azure = decode_database_token_expiry(bytes, TokenAuthKind::Azure);
});
