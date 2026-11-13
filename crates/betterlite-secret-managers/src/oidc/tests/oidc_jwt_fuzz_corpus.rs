use std::fs;
use std::path::Path;
use std::str::from_utf8;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::oidc::token_ttl::oidc_token_cache_ttl;

#[test]
fn replays_the_oidc_jwt_payload_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/oidc_jwt_payload");
    let ttls = fs::read_dir(corpus)
        .unwrap()
        .flat_map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            let wrapped = format!("e30.{}.c2ln", URL_SAFE_NO_PAD.encode(&bytes));
            let raw = from_utf8(&bytes)
                .ok()
                .map(|token| oidc_token_cache_ttl(token, 3600, 1_700_000_000));
            [
                raw,
                Some(oidc_token_cache_ttl(&wrapped, 3600, 1_700_000_000)),
            ]
        })
        .flatten()
        .collect::<Vec<_>>();
    assert!(ttls.len() >= 16);
    assert!(ttls.iter().all(|ttl| *ttl <= 3600));
}
