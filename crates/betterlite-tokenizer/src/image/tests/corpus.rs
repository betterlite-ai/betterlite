use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::str;

use betterlite_prompts::media::data_url::{MAX_MEDIA_BYTES, decode_data_url};
use rstest::rstest;

use crate::image::dimensions::image_dimensions;
use crate::image::header::image_type;
use crate::image::settings::Settings;

#[rstest]
fn replays_the_image_token_header_corpus(
    #[files("fuzz/corpus/image_token_header/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    black_box(image_type(&bytes));
    drop(image_dimensions(&bytes, &Settings::default()));
}

#[rstest]
fn replays_the_media_data_headers(
    #[files("../betterlite-prompts/fuzz/corpus/media_data/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    let Ok(decoded) = decode_data_url(text, MAX_MEDIA_BYTES) else {
        return;
    };
    drop(image_dimensions(&decoded, &Settings::default()));
}
