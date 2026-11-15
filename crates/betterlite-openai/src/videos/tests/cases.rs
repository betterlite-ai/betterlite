use rstest::rstest;

use super::auth::{auth_chain_case, content_variant_case};
use super::create::{create_multipart_case, input_reference_case};
use super::delete::{binary_content_case, delete_case};
use super::durations::{duration_case, invalid_seconds_case};
use super::list::list_and_status_case;
use super::remix::remix_case;

#[rstest]
#[case::video_create_multipart(0)]
#[case::input_reference(1)]
#[case::list_and_status(2)]
#[case::remix_request(3)]
#[case::delete_response(4)]
#[case::binary_content(5)]
#[case::invalid_seconds_kept_without_usage(6)]
#[case::duration_seconds_decimal(7)]
#[case::video_auth_chain(8)]
#[case::video_content_variant(9)]
fn videos_cases(#[case] case: u8) {
    match case {
        0 => create_multipart_case(),
        1 => input_reference_case(),
        2 => list_and_status_case(),
        3 => remix_case(),
        4 => delete_case(),
        5 => binary_content_case(),
        6 => invalid_seconds_case(),
        7 => duration_case(),
        8 => auth_chain_case(),
        _ => content_variant_case(),
    }
}
