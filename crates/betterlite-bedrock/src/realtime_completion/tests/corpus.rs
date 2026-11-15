use std::fs;
use std::io;
use std::path::Path;

use crate::realtime_completion::tool_input::decode_realtime_tool_input;

#[test]
fn replays_the_bedrock_services_realtime_completion_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/bedrock_services_realtime_completion");
    let mut seen = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let expected_ok = !(name.starts_with("invalid-")
                || name.starts_with("malformed-")
                || name.starts_with("truncated-"));
            let decoded = decode_realtime_tool_input(&bytes);
            assert_eq!(decoded.is_ok(), expected_ok, "{name}");
            if let Ok(raw) = decoded {
                serde_json::from_str::<Box<serde_json::value::RawValue>>(raw.get())
                    .unwrap_or_else(|error| panic!("{name} decodes to JSON: {error}"));
                if name.starts_with("valid-string-not-json") {
                    assert_eq!(raw.get(), "{}", "{name}");
                }
            }
            seen = seen.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seen >= 12, "expected committed seeds, found {seen}");
}
