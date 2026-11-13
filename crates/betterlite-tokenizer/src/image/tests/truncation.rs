use rstest::rstest;

use crate::image::dimensions::image_dimensions;
use crate::image::error::ImageError;
use crate::image::header::ImageType;

use super::support::DEFAULT;

#[rstest]
#[case::png_eight_bytes(b"\x89PNG\r\n\x1a\n", ImageType::Png)]
#[case::png_twenty_three_bytes(
    b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00",
    ImageType::Png
)]
#[case::gif_six_bytes(b"GIF89a", ImageType::Gif)]
#[case::gif_nine_bytes(b"GIF89a\x07\x00\t", ImageType::Gif)]
#[case::jpeg_three_bytes(b"\xff\xd8\xff", ImageType::Jpeg)]
#[case::jpeg_cut_segment(b"\xff\xd8\xff\xe0\x00\x07JF", ImageType::Jpeg)]
#[case::webp_vp8x_29(
    b"RIFF\x16\x00\x00\x00WEBPVP8X\n\x00\x00\x00\x00\x00\x00\x00+\x01\x00\xc7\x00",
    ImageType::Webp
)]
#[case::webp_vp8x_26(
    b"RIFF\x16\x00\x00\x00WEBPVP8X\n\x00\x00\x00\x00\x00\x00\x00+\x01",
    ImageType::Webp
)]
#[case::webp_vp8_29(
    b"RIFF\x16\x00\x00\x00WEBPVP8 \n\x00\x00\x00\x00\x00\x00\x9d\x01*,\x01\xc8",
    ImageType::Webp
)]
#[case::webp_vp8l_24(
    b"RIFF\x11\x00\x00\x00WEBPVP8L\x05\x00\x00\x00/+\xc11",
    ImageType::Webp
)]
fn image_token_geometry_behavior_tests_behavior(#[case] data: &[u8], #[case] expected: ImageType) {
    match image_dimensions(data, &DEFAULT) {
        Err(ImageError::Truncated { kind }) => assert_eq!(kind, expected),
        other => panic!("expected a {expected:?} truncation, got {other:?}"),
    }
}
