#![cfg(test)]
use crate::case::decode_http_case;
use std::{fs, path::Path};

#[test]
fn legacy_case_without_database() {
    decode_http_case(b"name: legacy\nconfig: config\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n").unwrap();
}

#[test]
fn replays_the_http_case_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/http_case");
    decode_http_case(&fs::read(root.join("postgres-valid.yaml")).unwrap()).unwrap();
}
