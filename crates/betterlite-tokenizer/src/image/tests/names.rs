use rstest::rstest;

use crate::image::header::ImageType;

#[rstest]
#[case::png(ImageType::Png, "png")]
#[case::gif(ImageType::Gif, "gif")]
#[case::jpeg(ImageType::Jpeg, "jpeg")]
#[case::heic(ImageType::Heic, "heic")]
#[case::webp(ImageType::Webp, "webp")]
fn image_token_geometry_behavior_tests_behavior(#[case] kind: ImageType, #[case] expected: &str) {
    assert_eq!(kind.as_str(), expected);
}
