#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::compare::helper::HelperOutput;
use crate::helpers::invoke;

fn seeds() -> Vec<(String, HelperCase)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds: Vec<(String, HelperCase)> = fs::read_dir(corpus)
        .expect("the helper corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("cometapi_record"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed");
            let case: HelperCase = serde_saphyr::from_str(&source).expect("a helper case");
            (path.display().to_string(), case)
        })
        .collect();
    seeds.sort_by(|left, right| left.0.cmp(&right.0));
    seeds
}

#[test]
fn helper_seeds_select_cometapi_record() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let rejected: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| {
            let decodes = serde_json::from_value::<super::Input>(case.call.input.clone()).is_ok();
            case.call.operation != "cometapi_record"
                || !decodes
                || validate_helper(Path::new("<helper>"), case).is_err()
        })
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(rejected.is_empty(), "{rejected:?}");
}

fn covers_chat(input: &super::Input) -> bool {
    input.steps.iter().any(|step| {
        matches!(
            step,
            super::Step::Options { .. }
                | super::Step::Request { .. }
                | super::Step::Url { .. }
                | super::Step::Chunk { .. }
        )
    })
}

fn covers_embedding(input: &super::Input) -> bool {
    input.steps.iter().any(|step| {
        matches!(
            step,
            super::Step::EmbeddingHeaders { .. }
                | super::Step::EmbeddingUrl { .. }
                | super::Step::EmbeddingOptions { .. }
                | super::Step::EmbeddingRequest { .. }
                | super::Step::EmbeddingReply { .. }
        )
    })
}

#[test]
fn helper_seeds_cover_chat_and_embedding_steps() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let inputs: Vec<super::Input> = seeds
        .iter()
        .map(|(_, case)| {
            serde_json::from_value(case.call.input.clone()).expect("a cometapi_record input")
        })
        .collect();
    let chat = inputs.iter().filter(|input| covers_chat(input)).count();
    let embedding = inputs
        .iter()
        .filter(|input| covers_embedding(input))
        .count();
    let outputs: Vec<HelperOutput> = seeds.iter().map(|(_, case)| invoke(&case.call)).collect();
    assert!(outputs.iter().all(|output| output.ok), "a seed did not run");
    assert!(chat > 0, "no seed covers a chat step");
    assert!(embedding > 0, "no seed covers an embedding step");
}
