#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::oracle_comparison_contracts::{Contracts, validate};
use crate::router_probes::{ProbeOperation, Reply, Request};
use crate::routing_probe_ipc::parse_message;

#[rstest]
fn replays_the_route_probe_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/route_probe");
    let mut seen = 0_usize;
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let bytes = fs::read(&path).unwrap();
        seen = seen.saturating_add(1);
        if name.starts_with("oracle-comparison-contracts-") {
            // the contracts seed parses, and only the valid one passes the production validator
            let contracts = parse_message::<Contracts>(&bytes).unwrap();
            let outcome = validate(&contracts.numeric, &contracts.expected_difference);
            assert_eq!(outcome.is_ok(), name.ends_with("-valid.json"), "{name}");
            continue;
        }
        if name.ends_with("-valid.json") {
            let request = parse_message::<Request>(&bytes).unwrap();
            assert!(
                matches!(request.operation, ProbeOperation::SingleCandidate(_)),
                "{name}"
            );
        }
        if name.ends_with("-invalid.json") {
            assert!(parse_message::<Request>(&bytes).is_err(), "{name}");
            assert!(parse_message::<Reply>(&bytes).is_err(), "{name}");
        }
    }
    assert!(seen > 0, "the route probe corpus is empty");
}
