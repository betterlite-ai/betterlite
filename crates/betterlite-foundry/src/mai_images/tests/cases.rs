use rstest::rstest;

use super::{params, replies, url, usage};

#[rstest]
#[case::mai_generation_edit_url(url::mai_generation_edit_url)]
#[case::default_dimensions(params::default_dimensions)]
#[case::known_and_custom_size(params::known_and_custom_size)]
#[case::dimension_and_pixel_limit(params::dimension_and_pixel_limit)]
#[case::n_one_only(params::n_one_only)]
#[case::drop_unsupported_fields(params::drop_unsupported_fields)]
#[case::usage_alias_priority(usage::usage_alias_priority)]
#[case::response_size(replies::response_size)]
fn mai_images_cases(#[case] check: fn()) {
    check();
}
