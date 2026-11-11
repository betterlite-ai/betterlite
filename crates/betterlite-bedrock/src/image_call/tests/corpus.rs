#![cfg(test)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::image_call::decode::decode_bedrock_image_reply;
use crate::image_models::family::Family;

const FAMILIES: [Family; 4] = [
    Family::Titan,
    Family::Stability1,
    Family::Stability3,
    Family::Nova,
];

#[test]
fn replays_the_bedrock_image_reply_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_image_reply");
    let paths: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(!paths.is_empty(), "no bedrock_image_reply seeds");
    paths
        .iter()
        .try_for_each(|path| {
            let bytes = fs::read(path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            if name.starts_with("valid-") {
                let family = family_from(name);
                assert!(decode_bedrock_image_reply(&bytes, family).is_ok(), "{name}");
            } else {
                assert!(
                    FAMILIES
                        .into_iter()
                        .all(|family| decode_bedrock_image_reply(&bytes, family).is_err()),
                    "{name}"
                );
            }
            Ok::<(), io::Error>(())
        })
        .unwrap();
}

fn family_from(name: &str) -> Family {
    if name.starts_with("valid-titan-") {
        Family::Titan
    } else if name.starts_with("valid-stability1-") {
        Family::Stability1
    } else if name.starts_with("valid-stability3-") {
        Family::Stability3
    } else if name.starts_with("valid-nova-") {
        Family::Nova
    } else {
        panic!("valid seed {name} names no family")
    }
}
