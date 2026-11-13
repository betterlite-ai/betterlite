use std::fs;
use std::io;
use std::path::Path;
use std::str;

use crate::web_identity::claims::decode_web_identity_claims;

#[test]
fn replays_the_bedrock_web_identity_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_web_identity");
    let mut seen = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            let bytes = fs::read(&path)?;
            seen = seen.saturating_add(1);
            match str::from_utf8(&bytes) {
                Ok(token) => {
                    let claims = decode_web_identity_claims(token);
                    assert_eq!(claims.is_some(), name.starts_with("claims-"), "{name}");
                }
                Err(_) => assert!(name.starts_with("malformed-"), "{name} is not UTF-8"),
            }
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seen > 0_usize, "the corpus is empty");
}
