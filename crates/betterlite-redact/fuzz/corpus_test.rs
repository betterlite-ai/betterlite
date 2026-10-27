#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_redact::structured::{
    Payload, mask_credentials_in_payload, mask_sensitive_structure,
    redact::redact_credentials_in_payload,
};

#[test]
fn replays_the_structured_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/structured_redaction");
    let paths = fs::read_dir(directory)
        .expect("corpus directory")
        .map(|entry| entry.expect("corpus entry").path())
        .collect::<Vec<_>>();
    paths
        .iter()
        .try_fold((), |(), path| {
            let bytes = fs::read(path).expect("corpus file");
            let value: Payload = serde_json::from_slice(&bytes).expect("valid corpus JSON");
            drop(mask_sensitive_structure(&value));
            drop(mask_credentials_in_payload(&value));
            drop(redact_credentials_in_payload(&value));
            Ok::<(), ()>(())
        })
        .expect("corpus replay");
}
