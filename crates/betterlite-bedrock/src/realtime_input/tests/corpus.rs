use std::fs;
use std::path::Path;

use crate::realtime_input::decode::decode_realtime_client_input;
use crate::realtime_input::error::InputError;

#[test]
fn replays_the_bedrock_services_realtime_input_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_realtime_input");
    let mut seeds = 0_usize;
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let outcome = decode_realtime_client_input(&fs::read(&path).unwrap());
        let category = match &outcome {
            Ok(_) => "valid",
            Err(InputError::InvalidJson { .. }) => "invalid-json",
            Err(InputError::Malformed { .. }) => "malformed",
        };
        let expected = ["valid", "boundary-valid", "invalid-json", "malformed"]
            .into_iter()
            .find(|prefix| name.starts_with(prefix))
            .map(|prefix| match prefix {
                "boundary-valid" => "valid",
                other => other,
            });
        assert_eq!(Some(category), expected, "seed {name}");
        seeds = seeds.saturating_add(1);
    }
    assert!(seeds >= 12, "the corpus holds only {seeds} seeds");
}
