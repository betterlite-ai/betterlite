#![cfg(test)]

mod oauth_request;
mod reply_decoding;
mod source_order;
mod support;
mod token_cache;

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::gigachat_tokens::reply::decode_token_reply;

#[rstest]
fn replays_token_reply(#[files("fuzz/corpus/gigachat_token_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();

    if let Ok(reply) = decode_token_reply(&bytes) {
        assert!(reply.token.expose_secret().len() <= bytes.len());
    }
}
