#![no_main]
use betterlite_auth::saml_provider::idp_metadata::decode_saml_metadata;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| drop(decode_saml_metadata(bytes)));
