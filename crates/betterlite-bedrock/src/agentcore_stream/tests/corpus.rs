use std::fs;
use std::io;
use std::path::Path;

use crate::agentcore_stream::line::decode_agentcore_line;

#[test]
fn replays_the_bedrock_services_agentcore_stream_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_agentcore_stream");
    let mut seen = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let mut buffer = Vec::new();
            let mut events = Vec::new();
            let outcome = decode_agentcore_line(&mut buffer, &bytes, &mut events);
            let expected_ok = !name.starts_with("invalid-");
            assert_eq!(outcome.is_ok(), expected_ok, "{name}");
            if expected_ok {
                let silent = name.starts_with("malformed-")
                    || name.starts_with("truncated-")
                    || matches!(name.as_str(), "boundary-empty" | "boundary-partial");
                assert_eq!(events.is_empty(), silent, "{name}");
                if name.starts_with("valid-") {
                    assert!(!events.is_empty(), "{name} emits an event");
                }
            }
            seen = seen.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seen >= 12, "expected committed seeds, found {seen}");
}
