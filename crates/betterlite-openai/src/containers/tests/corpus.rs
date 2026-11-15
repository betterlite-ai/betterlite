#![cfg(test)]

use std::fs;
use std::io::Error;
use std::path::PathBuf;

use crate::api::container_contracts::records::{ContainerFileListResponse, ContainerObject};
use crate::containers::replies::decode_container_reply;

#[test]
fn replays_the_openai_containers_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_containers");
    let mut paths = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "no container corpus seeds");
    paths
        .into_iter()
        .try_for_each(|path| {
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            if name.starts_with("valid-container-") {
                let decoded: Result<ContainerObject, _> = decode_container_reply(&bytes);
                assert!(decoded.is_ok(), "{name}: {decoded:?}");
            } else if name.starts_with("valid-file-list-") {
                let decoded: Result<ContainerFileListResponse, _> = decode_container_reply(&bytes);
                assert!(decoded.is_ok(), "{name}: {decoded:?}");
            } else {
                assert!(name.starts_with("invalid-"), "{name}: unknown seed class");
                let container: Result<ContainerObject, _> = decode_container_reply(&bytes);
                let files: Result<ContainerFileListResponse, _> = decode_container_reply(&bytes);
                assert!(container.is_err(), "{name} decoded as a container");
                assert!(files.is_err(), "{name} decoded as a file list");
            }
            Ok::<(), Error>(())
        })
        .unwrap();
}
