use rstest::rstest;

use super::{model_path, native_fields, non_v2, parameter_map, route, selection};

#[rstest]
#[case::dalle_gpt_model_selection(selection::dalle_gpt_model_selection)]
#[case::flux2_model_route(route::flux2_model_route)]
#[case::flux_model_path(model_path::flux_model_path)]
#[case::flux_size_parameter_map(parameter_map::flux_size_parameter_map)]
#[case::flux_native_fields(native_fields::flux_native_fields)]
#[case::flux_non_v2_policy(non_v2::flux_non_v2_policy)]
fn image_generation_cases(#[case] check: fn()) {
    check();
}
