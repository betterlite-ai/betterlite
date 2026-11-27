use rstest::rstest;

use super::lifecycle::{
    bodiless_s3_signature, delete_response_context, file_retrieve_delete, invalid_xml,
    s3_request_target_case,
};
use super::listing::{
    input_output_bucket_walk, last_modified_and_size, list_purpose_prefix, namespace_xml_list,
    pagination_continuation, unmanaged_key_filter,
};

#[rstest]
#[case::case_0_list_purpose_prefix(list_purpose_prefix)]
#[case::case_1_input_output_bucket_walk(input_output_bucket_walk)]
#[case::case_2_pagination_continuation(pagination_continuation)]
#[case::case_3_namespace_xml_list(namespace_xml_list)]
#[case::case_4_unmanaged_key_filter(unmanaged_key_filter)]
#[case::case_5_last_modified_and_size(last_modified_and_size)]
#[case::case_6_file_retrieve_delete(file_retrieve_delete)]
#[case::case_7_invalid_xml(invalid_xml)]
#[case::case_8_delete_response_context(delete_response_context)]
#[case::case_9_s3_request_target(s3_request_target_case)]
#[case::case_10_bodiless_s3_signature(bodiless_s3_signature)]
fn s3_listing_cases(#[case] check: fn()) {
    check();
}
