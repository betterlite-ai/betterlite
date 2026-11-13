use rstest::rstest;

use crate::image::tokens::{Mode, calculate_img_tokens};

use super::support::{
    BAD_BASE64_URL, DEFAULT, GIF_URL, HEIC_URL, JPEG_URL, NO_PREFIX_URL, PNG_1X1_URL, PNG_4096_URL,
    WEBP_VP8X_URL, WEBP_VP8X_WIDE_URL,
};

#[rstest]
#[case::png_low(PNG_1X1_URL, Mode::Low, 85, false, 85)]
#[case::png_auto(PNG_1X1_URL, Mode::Auto, 85, false, 85)]
#[case::png_high(PNG_1X1_URL, Mode::High, 85, false, 255)]
#[case::png_large_high(PNG_4096_URL, Mode::High, 85, false, 765)]
#[case::png_large_custom_base(PNG_4096_URL, Mode::High, 100, false, 900)]
#[case::gif_high(GIF_URL, Mode::High, 85, false, 255)]
#[case::jpeg_high(JPEG_URL, Mode::High, 85, false, 255)]
#[case::webp_high(WEBP_VP8X_URL, Mode::High, 85, false, 255)]
#[case::webp_wide_high(WEBP_VP8X_WIDE_URL, Mode::High, 85, false, 1105)]
#[case::heic_high(HEIC_URL, Mode::High, 85, false, 255)]
#[case::default_count(PNG_1X1_URL, Mode::High, 85, true, 250)]
#[case::default_count_beats_decode(NO_PREFIX_URL, Mode::High, 85, true, 250)]
#[case::low_skips_decode(NO_PREFIX_URL, Mode::Low, 85, false, 85)]
#[case::auto_skips_decode(BAD_BASE64_URL, Mode::Auto, 85, false, 85)]
fn image_token_geometry_behavior_tests_behavior(
    #[case] data: &str,
    #[case] mode: Mode,
    #[case] base_tokens: u64,
    #[case] use_default_image_token_count: bool,
    #[case] expected: u64,
) {
    match calculate_img_tokens(
        data,
        mode,
        base_tokens,
        use_default_image_token_count,
        &DEFAULT,
    ) {
        Ok(tokens) => assert_eq!(tokens, expected),
        Err(error) => panic!("expected {expected} tokens, got {error:?}"),
    }
}
