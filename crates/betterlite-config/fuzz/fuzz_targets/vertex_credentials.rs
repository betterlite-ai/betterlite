#![no_main]

use betterlite_config::sections::vertex_credential_sources::credential_source::decode_vertex_credentials;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = decode_vertex_credentials(data);
});
