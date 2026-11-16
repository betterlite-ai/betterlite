#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::api::video_records::helpers::{get_custom_provider_from_data, video_reference_to_id};

#[rstest]
fn replays_the_video_reference_to_id_corpus(
    #[files("fuzz/corpus/video_reference_to_id/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    drop(video_reference_to_id(&Json::Text(text.to_owned())));
    let Ok(value) = serde_json::from_str::<Json>(text) else {
        return;
    };
    drop(video_reference_to_id(&value));
    if let Json::Object(fields) = &value {
        drop(get_custom_provider_from_data(fields));
    }
}
