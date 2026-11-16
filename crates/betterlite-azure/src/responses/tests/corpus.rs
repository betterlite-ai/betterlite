#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use crate::responses::decode::decode_azure_response_resource;

#[test]
fn replays_the_azure_responses_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_responses");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let decoded = decode_azure_response_resource(&bytes);
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
