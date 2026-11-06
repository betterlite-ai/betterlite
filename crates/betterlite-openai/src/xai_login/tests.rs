#![cfg(test)]

mod callback;
mod listener;
mod login;
mod pkce;
mod support;

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::xai_login::callback::decode_xai_callback;

#[rstest]
fn replays_xai_callback_corpus(#[files("fuzz/corpus/xai_callback/*")] path: PathBuf) {
    let _request = decode_xai_callback(&fs::read(path).unwrap());
}
