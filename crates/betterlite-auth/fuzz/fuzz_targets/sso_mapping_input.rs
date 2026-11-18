#![no_main]

use betterlite_auth::sso_mappings::headers::decode_generic_headers;
use betterlite_auth::sso_mappings::roles::decode_role_mapping_literal;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Some((&tag, payload)) = bytes.split_first() else {
        return;
    };
    match tag {
        0 => drop(decode_role_mapping_literal(
            payload,
            Some("groups"),
            Some("proxy_admin"),
        )),
        1 => drop(decode_generic_headers(Some(payload))),
        _other => {}
    }
});
