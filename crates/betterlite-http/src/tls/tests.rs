#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use crate::tls::parse_ca_pem;
use rstest::rstest;

#[rstest]
fn replays_the_http_policy_corpus(#[files("fuzz/corpus/http_policy/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _result = parse_ca_pem(&bytes);
}

mod ca_policy;
mod curve_policy;
mod deployment_policy;
mod local_tls;

mod trust;
