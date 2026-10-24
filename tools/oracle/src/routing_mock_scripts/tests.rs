#![cfg(test)]

use std::fs;
use std::io::{self, Read as _, Write as _};
use std::net::TcpStream;
use std::path::PathBuf;

use super::{CallOutcome, ProviderCall, parse_script};
use crate::mock::MockProvider;

mod routing_mock_scripts_behaviors;
mod routing_mock_scripts_boundaries;

const OK: &str = r#"{"then":{"answer":{"status":200,"headers":{},"body":{}}}}"#;
const BUSY: &str = r#"{"then":{"answer":{"status":503,"headers":{},"body":{}}}}"#;
const SLOW: &str =
    r#"{"wait":{"delay":{"millis":40}},"then":{"answer":{"status":200,"headers":{},"body":{}}}}"#;
const PAIR: &str = r#"{"wait":{"barrier":{"name":"pair"}},"then":{"answer":{"status":200,"headers":{},"body":{}}}}"#;
const ANSWERED: CallOutcome = CallOutcome::Answered { status: 200 };

fn script(deadline_ms: u64, replies: &[&str]) -> Vec<u8> {
    format!(
        r#"{{"deadline_ms":{deadline_ms},"replies":[{}]}}"#,
        replies.join(",")
    )
    .into_bytes()
}

fn start(deadline_ms: u64, replies: &[&str]) -> MockProvider {
    MockProvider::start(parse_script(&script(deadline_ms, replies)).unwrap()).unwrap()
}

fn connect(mock: &MockProvider, path: &str) -> TcpStream {
    let mut stream = TcpStream::connect(mock.base_url.strip_prefix("http://").unwrap()).unwrap();
    write!(stream, "POST {path} HTTP/1.1\r\nHost: mock\r\nAuthorization: Bearer sk-secret\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
    stream
}

fn call(mock: &MockProvider, path: &str) -> String {
    let mut received = String::new();
    let _read = connect(mock, path).read_to_string(&mut received);
    received.get(9..12).unwrap_or_default().to_owned()
}

fn steps(trace: &[ProviderCall]) -> Vec<(&str, CallOutcome)> {
    trace
        .iter()
        .map(|call| (call.request.path.as_str(), call.outcome))
        .collect()
}

fn seed(kind: &str) -> Vec<u8> {
    let name = format!("fuzz/corpus/mock_script/routing-mock-scripts-{kind}.json");
    fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name)).unwrap()
}

#[test]
fn replays_the_mock_script_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/mock_script");
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            drop(parse_script(&fs::read(entry?.path())?));
            Ok::<(), io::Error>(())
        })
        .unwrap();
}
