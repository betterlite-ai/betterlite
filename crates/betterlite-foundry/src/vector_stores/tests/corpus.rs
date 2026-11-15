use std::fs;
use std::path::{Path, PathBuf};

use crate::vector_stores::reply::{AzureSearchStoreReply, decode_azure_search_reply};
use crate::vector_stores::search::SearchLogging;

#[test]
fn replays_the_foundry_vector_stores_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/foundry_vector_stores");
    let paths: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!paths.is_empty());
    let logging = SearchLogging::default();
    let failures: Vec<String> = paths
        .iter()
        .filter_map(|path| {
            let bytes = fs::read(path).expect("corpus file");
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("name");
            let reply = AzureSearchStoreReply {
                status_code: 200,
                headers: &[],
                body: &bytes,
                logging: &logging,
            };
            let decoded = decode_azure_search_reply(&reply);
            (decoded.is_ok() != name.starts_with("ok-")).then(|| name.to_owned())
        })
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}
