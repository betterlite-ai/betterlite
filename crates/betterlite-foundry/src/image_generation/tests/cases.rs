use rstest::rstest;

use super::{model_path, native_fields, non_v2, parameter_map, route, selection};

#[rstest]
#[case::case_1_dalle_gpt_model_selection(selection::dalle_gpt_model_selection)]
#[case::case_2_flux2_model_route(route::flux2_model_route)]
#[case::case_3_flux_model_path(model_path::flux_model_path)]
#[case::case_4_flux_size_parameter_map(parameter_map::flux_size_parameter_map)]
#[case::case_5_flux_native_fields(native_fields::flux_native_fields)]
#[case::case_6_flux_non_v2_policy(non_v2::flux_non_v2_policy)]
fn image_generation_cases(#[case] check: fn()) {
    check();
}
