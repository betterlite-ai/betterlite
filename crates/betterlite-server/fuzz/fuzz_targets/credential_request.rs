#![no_main]

use betterlite_db::resource_rows::credential::{CreateCredentialItem, UpdateCredentialItem};
use betterlite_server::credentials::request::decode_credential_request;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    drop(decode_credential_request::<CreateCredentialItem>(body));
    drop(decode_credential_request::<UpdateCredentialItem>(body));
});
