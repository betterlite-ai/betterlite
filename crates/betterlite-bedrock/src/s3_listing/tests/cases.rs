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
#[case::list_purpose_prefix(list_purpose_prefix)]
#[case::input_output_bucket_walk(input_output_bucket_walk)]
#[case::pagination_continuation(pagination_continuation)]
#[case::namespace_xml_list(namespace_xml_list)]
#[case::unmanaged_key_filter(unmanaged_key_filter)]
#[case::last_modified_and_size(last_modified_and_size)]
#[case::file_retrieve_delete(file_retrieve_delete)]
#[case::invalid_xml(invalid_xml)]
#[case::delete_response_context(delete_response_context)]
#[case::s3_request_target(s3_request_target_case)]
#[case::bodiless_s3_signature(bodiless_s3_signature)]
fn s3_listing_cases(#[case] check: fn()) {
    check();
}
