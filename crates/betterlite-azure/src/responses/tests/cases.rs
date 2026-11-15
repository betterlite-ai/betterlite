#![cfg(test)]

use rstest::rstest;

use super::{api_version, cancel, capabilities, o_series, reasoning, resources, strip, websocket};

#[rstest]
#[case::responses_base_and_api_version(api_version::responses_base_and_api_version)]
#[case::strip_azure_model_prefix(strip::strip_azure_model_prefix)]
#[case::o_series_options(o_series::o_series_options)]
#[case::reasoning_null_fields(reasoning::reasoning_null_fields)]
#[case::websocket_model_query(websocket::websocket_model_query)]
#[case::resource_id_before_query(resources::resource_id_before_query)]
#[case::cancel_response(cancel::cancel_response)]
#[case::azure_responses_capabilities(capabilities::azure_responses_capabilities)]
fn responses_cases(#[case] case: fn()) {
    case();
}
