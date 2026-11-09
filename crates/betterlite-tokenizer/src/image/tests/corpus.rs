use std::fs;
use std::hint::black_box;
use std::path::PathBuf;

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
