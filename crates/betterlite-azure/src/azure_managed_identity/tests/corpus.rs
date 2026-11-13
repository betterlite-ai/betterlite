use std::fs;
use std::io;
use std::path::Path;

use crate::azure_managed_identity::decode::{ImdsReply, ImdsTokenError, decode_imds_token};

fn category(bytes: &[u8]) -> &'static str {
    match decode_imds_token(bytes) {
        Ok(ImdsReply::Token(_)) => "ok",
        Ok(ImdsReply::Error { .. }) => "error",
        Ok(ImdsReply::Missing) => "missing",
        Err(ImdsTokenError::Json(_)) => "syntax",
        Err(
            ImdsTokenError::NotAnObject
            | ImdsTokenError::FieldType { .. }
            | ImdsTokenError::Integer { .. },
        ) => "value",
    }
}

#[test]
fn replays_the_azure_managed_identity_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_managed_identity");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let directory_entry = entry?;
            if directory_entry.file_type()?.is_dir() {
                return Ok(());
            }
            let path = directory_entry.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let prefix = name.split('_').next().unwrap();
            assert_eq!(category(&bytes), prefix, "{name}");
            replayed = replayed.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(replayed > 0_usize);
}
