use std::str;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::support::{
    JPEG_BASE64, PNG_BASE64, bmp_base64, bmp_data_url, jpeg_data_url, oversized_bmp_data_url,
    png_data_url,
};
use crate::ollama_generate::request::convert_image;

fn decoded(image: &str) -> Vec<u8> {
    STANDARD.decode(image).unwrap()
}

#[test]
fn jpeg_payload_passes_through_unchanged() {
    assert_eq!(convert_image(&jpeg_data_url()).unwrap(), JPEG_BASE64);
}

#[test]
fn png_payload_passes_through_unchanged() {
    assert_eq!(convert_image(&png_data_url()).unwrap(), PNG_BASE64);
}

#[test]
fn a_space_inside_the_base64_is_kept_when_the_bytes_are_jpeg() {
    let mut wrapped = String::from(JPEG_BASE64);
    wrapped.insert(10, ' ');
    assert_eq!(convert_image(&wrapped).unwrap(), wrapped);
}

#[test]
fn newline_wrapped_base64_is_kept_when_the_bytes_are_png() {
    let wrapped = PNG_BASE64
        .as_bytes()
        .chunks(20)
        .map(str::from_utf8)
        .collect::<Result<Vec<&str>, _>>()
        .unwrap()
        .join("\n");
    assert_eq!(convert_image(&wrapped).unwrap(), wrapped);
}

#[test]
fn a_data_prefix_without_an_image_type_is_stripped() {
    let image = format!("data:application/octet-stream;base64,{JPEG_BASE64}");
    assert_eq!(convert_image(&image).unwrap(), JPEG_BASE64);
}

#[test]
fn undecodable_base64_returns_the_original_image() {
    assert_eq!(convert_image("!!!").unwrap(), "!!!");
}

#[test]
fn non_ascii_base64_returns_the_original_image() {
    assert_eq!(convert_image("QUJD\u{e9}").unwrap(), "QUJD\u{e9}");
}

#[test]
fn a_payload_that_is_not_an_image_returns_the_original() {
    assert_eq!(convert_image("QUJD").unwrap(), "QUJD");
}

#[test]
fn empty_input_returns_the_original() {
    assert_eq!(convert_image("").unwrap(), "");
}

#[test]
fn unpadded_base64_returns_the_original() {
    let unpadded = JPEG_BASE64.trim_end_matches('=');
    assert_eq!(convert_image(unpadded).unwrap(), unpadded);
}

#[test]
fn an_unpadded_data_url_is_returned_unchanged() {
    let unpadded = JPEG_BASE64.trim_end_matches('=');
    let image = format!("data:image/jpeg;base64,{unpadded}");
    assert_eq!(convert_image(&image).unwrap(), image);
}

#[test]
fn extra_padding_is_tolerated() {
    let padded = format!("{JPEG_BASE64}====");
    assert_eq!(convert_image(&padded).unwrap(), padded);
}

#[test]
fn other_image_formats_are_re_encoded_as_jpeg() {
    let found = convert_image(&bmp_data_url()).unwrap();
    assert_ne!(found, bmp_base64());
    assert!(decoded(&found).starts_with(&[0xFF, 0xD8, 0xFF]));
}

#[test]
fn a_space_in_a_data_url_is_dropped_while_the_payload_is_kept() {
    let mut wrapped = String::from(JPEG_BASE64);
    wrapped.insert(10, ' ');
    let image = format!("data:image/jpeg;base64,{wrapped}");
    assert_eq!(convert_image(&image).unwrap(), wrapped);
}

#[test]
fn an_image_that_declares_oversized_dimensions_is_returned_unchanged() {
    let image = oversized_bmp_data_url();
    assert_eq!(convert_image(&image).unwrap(), image);
}

#[test]
fn a_non_ascii_data_url_is_returned_unchanged() {
    let image = format!("data:image/jpeg;base64,{JPEG_BASE64}\u{e9}");
    assert_eq!(convert_image(&image).unwrap(), image);
}

#[test]
fn a_bmp_without_a_data_prefix_is_re_encoded_too() {
    let found = convert_image(&bmp_base64()).unwrap();
    assert_ne!(found, bmp_base64());
    assert!(decoded(&found).starts_with(&[0xFF, 0xD8, 0xFF]));
}
