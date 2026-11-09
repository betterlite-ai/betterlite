use rstest::rstest;

use crate::image::dimensions::image_dimensions;
use crate::image::settings::Settings;

use super::support::{CUSTOM_DEFAULTS, DEFAULT};

#[rstest]
#[case::png_1x1(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01", &DEFAULT, (1, 1))]
#[case::png_4096(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x10\x00\x00\x00\x10\x00", &DEFAULT, (4096, 4096))]
#[case::png_max_dims(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\xff\xff\xff\xff\x00\x00\x00\x02", &DEFAULT, (0xFFFF_FFFF, 2))]
#[case::gif_7x9(b"GIF89a\x07\x00\t\x00", &DEFAULT, (7, 9))]
#[case::jpeg_11x13(b"\xff\xd8\xff\xe0\x00\x07JFIF\x00\xff\xc0\x00\x11\x08\x00\r\x00\x0b", &DEFAULT, (11, 13))]
#[case::jpeg_skips_segments(b"\xff\xd8\xff\xe0\x00\x07JFIF\x00\xff\xc4\x00\x04\x00\x00\xff\xc0\x00\x11\x08\x00\x02\x00\x03", &DEFAULT, (3, 2))]
#[case::jpeg_fill_bytes(b"\xff\xd8\xff\xff\xff\xc2\x00\x11\x08\x00\x04\x00\x05", &DEFAULT, (5, 4))]
#[case::jpeg_excluded_marker(b"\xff\xd8\xff\xc8\x00\x04\x00\x00\xff\xc0\x00\x11\x08\x00\x06\x00\x07", &DEFAULT, (7, 6))]
#[case::jpeg_zero_length(b"\xff\xd8\xff\xe0\x00\x00\x04\x00\x00\xff\xc0\x00\x11\x08\x00\x02\x00\x03", &DEFAULT, (3, 2))]
#[case::jpeg_dac_marker(b"\xff\xd8\xff\xcc\x00\x04\x00\x00\xff\xc0\x00\x11\x08\x00\x02\x00\x03", &DEFAULT, (3, 2))]
#[case::jpeg_last_frame_marker(b"\xff\xd8\xff\xcf\x00\x11\x08\x00\x05\x00\x06", &DEFAULT, (6, 5))]
#[case::jpeg_other_markers(b"\xff\xd8\xff\xbf\x00\x04\x00\x00\xff\xd0\x00\x02\xff\xc0\x00\x11\x08\x00\x08\x00\x09", &DEFAULT, (9, 8))]
#[case::webp_vp8x(b"RIFF\x16\x00\x00\x00WEBPVP8X\n\x00\x00\x00\x00\x00\x00\x00+\x01\x00\xc7\x00\x00", &DEFAULT, (300, 200))]
#[case::webp_vp8_masked(b"RIFF\x16\x00\x00\x00WEBPVP8 \n\x00\x00\x00\x00\x00\x00\x9d\x01*,\xc1,\xc1", &DEFAULT, (300, 300))]
#[case::webp_vp8(b"RIFF\x16\x00\x00\x00WEBPVP8 \n\x00\x00\x00\x00\x00\x00\x9d\x01*,\x01\xc8\x00", &DEFAULT, (300, 200))]
#[case::webp_vp8l(b"RIFF\x11\x00\x00\x00WEBPVP8L\x05\x00\x00\x00/+\xc11\x00", &DEFAULT, (300, 200))]
#[case::webp_vp8l_alpha_bit(b"RIFF\x11\x00\x00\x00WEBPVP8L\x05\x00\x00\x00/+\xc11\x10", &DEFAULT, (300, 200))]
#[case::webp_unknown_chunk(b"RIFF\x0c\x00\x00\x00WEBPXXXX\x00\x00\x00\x00", &DEFAULT, (300, 300))]
#[case::webp_unknown_custom(b"RIFF\x0c\x00\x00\x00WEBPXXXX\x00\x00\x00\x00", &CUSTOM_DEFAULTS, (640, 480))]
#[case::webp_short_chunk(b"RIFF\x00\x00\x00\x00WEBPVP8", &DEFAULT, (300, 300))]
#[case::heic(b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00", &DEFAULT, (300, 300))]
#[case::heic_custom(b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00", &CUSTOM_DEFAULTS, (640, 480))]
#[case::unknown(b"not an image", &DEFAULT, (300, 300))]
#[case::unknown_custom(b"not an image", &CUSTOM_DEFAULTS, (640, 480))]
#[case::empty(b"", &DEFAULT, (300, 300))]
fn image_token_geometry_behavior_tests_behavior(
    #[case] data: &[u8],
    #[case] settings: &Settings,
    #[case] expected: (u32, u32),
) {
    match image_dimensions(data, settings) {
        Ok(size) => assert_eq!(size, expected),
        Err(error) => panic!("expected {expected:?}, got {error:?}"),
    }
}
