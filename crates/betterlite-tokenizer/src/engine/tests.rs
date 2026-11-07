#![cfg(test)]

mod engines_follow_litellm;
mod hub_download_follows_huggingface_hub;
mod limits_hold;
mod selection_follows_litellm;
mod support;

use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use rstest::rstest;

use crate::engine::assets;
use crate::engine::huggingface::HuggingFaceEngine;
use crate::engine::tiktoken::{Encoding, TiktokenEngine};

fn p50k() -> TiktokenEngine {
    let data = assets::bundled(Encoding::P50kBase).unwrap();
    TiktokenEngine::from_bpe_file(Encoding::P50kBase, &data).unwrap()
}

#[rstest]
fn replays_the_tokenizer_asset_corpus(#[files("fuzz/corpus/tokenizer_asset/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _vocabulary = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &bytes);
    let _tokenizer = HuggingFaceEngine::from_bytes(&bytes);
}

#[rstest]
fn replays_the_tokenizer_input_corpus(#[files("fuzz/corpus/tokenizer_input/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let engine = p50k();
    if let Ok(text) = from_utf8(&bytes)
        && let Ok(ids) = engine.encode(text)
    {
        let _round_trip = engine.decode(&ids);
        let half = ids.len().checked_div(2).unwrap_or(0);
        let _prefix = engine.decode(ids.get(..half).unwrap_or(&[]));
    }
    let ids: Vec<u32> = bytes
        .chunks(4)
        .filter(|chunk| chunk.len() == 4)
        .map(|chunk| u32::from_le_bytes(<[u8; 4]>::try_from(chunk).unwrap_or([0; 4])))
        .collect();
    let _decoded = engine.decode(&ids);
}
