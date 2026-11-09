use rstest::rstest;
use temp_env::with_var;

use crate::image::settings::Settings;

#[rstest]
fn image_token_geometry_behavior_tests_behavior() {
    with_var("DEFAULT_IMAGE_TOKEN_COUNT", Some("500"), || {
        assert_eq!(Settings::default().default_token_count, 250);
    });
}
