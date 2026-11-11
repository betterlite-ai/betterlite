use std::fs;
use std::io;
use std::path::Path;

use crate::vector_stores::response::decode_knowledge_base_reply;

#[test]
fn replays_the_bedrock_services_vector_stores_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_vector_stores");
    let mut seen = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let outcome = decode_knowledge_base_reply(200, &[], &bytes, "query");
            let expected_ok = name.starts_with("valid-") || name.starts_with("boundary-valid-");
            assert_eq!(outcome.is_ok(), expected_ok, "{name}");
            seen = seen.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seen >= 12, "expected committed seeds, found {seen}");
}
