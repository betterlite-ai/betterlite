#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::convert::{ConnectorOutcome, convert_connector_entries};
use super::request::ConnectorImportRequest;
use crate::server_payload::rules::StdioPolicy;

fn corpus_policy() -> StdioPolicy {
    StdioPolicy {
        enabled: true,
        allowed_commands: ["node".to_owned(), "npx".to_owned()]
            .into_iter()
            .collect::<BTreeSet<String>>(),
    }
}

#[test]
fn replays_the_connector_import_request_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/connector_import_request");
    let policy = corpus_policy();
    let mut seen = 0_u32;
    let mut converted = 0_usize;
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let bytes = fs::read(&path).unwrap();
        seen += 1_u32;
        let Ok(request) = ConnectorImportRequest::from_slice(&bytes) else {
            continue;
        };
        let outcomes = convert_connector_entries(&request, &policy);
        converted += outcomes
            .into_iter()
            .filter(|outcome| matches!(outcome, ConnectorOutcome::Converted(_)))
            .count();
    }
    assert!(seen >= 3_u32, "expected committed seeds, found {seen}");
    assert!(converted >= 1_usize, "expected a converted connector");
}
