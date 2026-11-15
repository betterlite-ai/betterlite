use std::fs;
use std::io;
use std::path::Path;

use crate::realtime_output::metadata::decode_generation_metadata;

#[test]
fn replays_the_bedrock_services_realtime_output_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_realtime_output");
    let mut seeds = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let text = fs::read_to_string(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let decoded = decode_generation_metadata(&text);
            if name.starts_with("valid-stage-") {
                assert!(matches!(decoded, Ok(Some(_))), "{name}");
            } else if name.starts_with("valid-none-") {
                assert!(matches!(decoded, Ok(None)), "{name}");
            } else {
                assert!(name.starts_with("malformed-"), "{name}");
                assert!(decoded.is_err(), "{name}");
            }
            seeds = seeds.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seeds >= 18, "expected committed seeds, found {seeds}");
}
