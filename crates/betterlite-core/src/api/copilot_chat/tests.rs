#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_http::proxy::UpstreamResponse;
use http::HeaderMap;
use uuid::Uuid;

use super::reply::transform_response;

mod config;
mod messages;
mod native;
mod raw;
mod support;

#[rstest::rstest]
fn replays_the_copilot_chat_reply_corpus(
    #[files("fuzz/corpus/copilot_chat_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a corpus file");
    let response = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: bytes,
    };
    drop(transform_response(response, None, false, Uuid::new_v4));
}
