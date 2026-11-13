use std::fs;
use std::path::Path;

use crate::manus_files::reply::{decode_create_file, decode_deleted_file, decode_file_list};

use super::support::NOW;

const ACCEPTED: [&str; 3] = ["boundary.json", "missing-fields.json", "valid.json"];

#[test]
fn replays_the_manus_files_reply_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/manus_files_reply");
    let entries: Vec<(String, Vec<u8>)> = fs::read_dir(corpus)
        .expect("the corpus folder exists")
        .map(|entry| {
            let path = entry.expect("the corpus entry reads").path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("the corpus file name is UTF-8")
                .to_owned();
            let bytes = fs::read(path).expect("the corpus file reads");
            (name, bytes)
        })
        .collect();
    assert!(entries.len() >= ACCEPTED.len());
    let outcomes: Vec<(String, bool)> = entries
        .iter()
        .map(|(name, bytes)| {
            let create = decode_create_file(bytes, NOW);
            let _list = decode_file_list(bytes, NOW);
            let _deleted = decode_deleted_file(bytes);
            (name.clone(), create.is_ok())
        })
        .collect();
    let agrees = outcomes
        .iter()
        .all(|(name, accepted)| ACCEPTED.contains(&name.as_str()) == *accepted);
    assert!(agrees, "{outcomes:?}");
}
