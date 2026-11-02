#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde_json::Value;

use super::parse_scenario;
use crate::case::{BodyKind, Request, UpstreamAnswer};
use crate::http::Response;
use crate::mock::ProviderRequest;

mod routing_scenarios_behaviors;
mod routing_scenarios_boundaries;

fn request(path: &str) -> Request {
    Request {
        method: "POST".to_owned(),
        path: path.to_owned(),
        headers: BTreeMap::new(),
        body: Value::Null,
        body_kind: BodyKind::Json,
    }
}

fn tagged_body(tag: &str) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("tag".to_owned(), Value::String(tag.to_owned()));
    Value::Object(body)
}

fn answer(status: u16, tag: &str) -> UpstreamAnswer {
    UpstreamAnswer {
        status,
        headers: BTreeMap::new(),
        body: Some(tagged_body(tag)),
        events: None,
    }
}

fn response(status: u16, tag: &str) -> Response {
    Response {
        status,
        headers: BTreeMap::new(),
        body: tagged_body(tag),
    }
}

fn provider_request(header: &str, value: &str) -> ProviderRequest {
    ProviderRequest {
        method: "POST".to_owned(),
        path: "/v1/chat/completions".to_owned(),
        headers: BTreeMap::from([(header.to_owned(), value.to_owned())]),
        body: Value::Null,
    }
}

#[test]
fn replays_the_scenario_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/scenario");
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            drop(parse_scenario(&fs::read(entry?.path())?));
            Ok::<(), io::Error>(())
        })
        .unwrap();
}
