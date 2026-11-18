#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::answers::completion;
use super::input::Input;

#[test]
fn helper_seeds_select_foundry_anthropic() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("foundry_anthropic"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .unwrap()
                .call
                .operation
                == "foundry_anthropic"
    }));
    assert_stream_events();
}

fn assert_stream_events() {
    let path = Path::new(ORACLE_DIR)
        .join("cases/helpers/providers/foundry-anthropic/chat-stream-native-events.yaml");
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let mut input: Input = serde_json::from_value(case.call.input).unwrap();
    let answer = completion(&input).unwrap();
    assert_eq!(answer["stream"]["text"], "hello");
    assert_eq!(answer["stream"]["provider"], "azure_ai");
    assert_eq!(
        answer["stream"]["finish_reasons"],
        serde_json::json!(["stop"])
    );
    input.upstream_lines.retain(|line| !line.contains("hello"));
    let without_delta = completion(&input).unwrap();
    assert_eq!(without_delta["stream"]["text"], "");
    assert_ne!(answer, without_delta);
}
