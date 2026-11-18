#![no_main]

use betterlite_auth::oidc_token_exchange::decode::OidcResponseKind;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Some((&tag, payload)) = bytes.split_first() else {
        return;
    };
    if let Ok(kind) = OidcResponseKind::try_from(tag) {
        drop(kind.decode(payload));
    }
});
