use std::path::PathBuf;

use rstest::rstest;

use crate::sso_mappings::headers::decode_generic_headers;
use crate::sso_mappings::roles::decode_role_mapping_literal;

// the seeds only have to be survived; what they decode to is pinned by the oracle cases
#[rstest]
fn replays_the_sso_mapping_input_corpus(#[files("fuzz/corpus/sso_mapping_input/*")] path: PathBuf) {
    let bytes = std::fs::read(&path).expect("the corpus seed");
    let Some((&tag, payload)) = bytes.split_first() else {
        panic!("the seed {path:?} carries a tag byte");
    };
    match tag {
        0 => drop(decode_role_mapping_literal(
            payload,
            Some("groups"),
            Some("proxy_admin"),
        )),
        1 => drop(decode_generic_headers(Some(payload))),
        _other => panic!("the seed {path:?} has an unknown tag"),
    }
}

#[test]
fn the_corpus_covers_both_fuzz_paths() {
    let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/sso_mapping_input");
    let tags: Vec<u8> = std::fs::read_dir(folder)
        .expect("the corpus folder")
        .map(|entry| std::fs::read(entry.expect("a corpus entry").path()).expect("the seed"))
        .map(|bytes| bytes[0])
        .collect();
    assert!(tags.contains(&0));
    assert!(tags.contains(&1));
}
