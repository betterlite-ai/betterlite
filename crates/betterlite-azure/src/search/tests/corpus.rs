#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use crate::search::reply::{AzureSearchReply, decode_grounding_reply};

#[test]
fn replays_the_azure_search_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_search");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let reply = AzureSearchReply {
                status_code: 200,
                headers: &[],
                body: &bytes,
            };
            let decoded = decode_grounding_reply(&reply);
            match name.split('_').next().unwrap() {
                "valid" | "boundary" => assert!(decoded.is_ok(), "{name}"),
                "malformed" | "truncated" => assert!(decoded.is_err(), "{name}"),
                other => panic!("unexpected corpus category {other}"),
            }
            replayed += 1_usize;
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(replayed > 0_usize);
}
