use rstest::rstest;

use crate::image::header::{ImageType, image_type};

#[rstest]
#[case::png(b"\x89PNG\r\n\x1a\n", Some(ImageType::Png))]
#[case::gif87(b"GIF87a", Some(ImageType::Gif))]
#[case::gif89(b"GIF89a", Some(ImageType::Gif))]
#[case::gif_wrong_version(b"GIF8xx", None)]
#[case::gif_prefix_only(b"GIF8", None)]
#[case::jpeg(b"\xff\xd8\xff", Some(ImageType::Jpeg))]
#[case::jpeg_prefix_only(b"\xff\xd8", None)]
#[case::heic(b"\x00\x00\x00\x18ftypheic", Some(ImageType::Heic))]
#[case::heic_seven_bytes(b"\x00\x00\x00\x18fty", None)]
#[case::webp(b"RIFF\x00\x00\x00\x00WEBPVP8X", Some(ImageType::Webp))]
#[case::riff_not_webp(b"RIFF\x00\x00\x00\x00XXXX", None)]
#[case::webp_short_chunk(b"RIFF\x00\x00\x00\x00WEB", None)]
#[case::unknown(b"not an image", None)]
#[case::empty(b"", None)]
fn image_token_geometry_behavior_tests_behavior(
    #[case] data: &[u8],
    #[case] expected: Option<ImageType>,
) {
    assert_eq!(image_type(data), expected);
}
