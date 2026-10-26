#![cfg(test)]

use super::{
    ResponseAPIUsage, ResponseObject, TokenDetails, assert_filtered_counts, reconcile_io_usage,
    usage,
};

#[test]
fn response_api_billable_counts() {
    let value = ResponseAPIUsage {
        input_tokens: 100,
        output_tokens: 20,
        total_tokens: 120,
        input_tokens_details: Some(TokenDetails { cached_tokens: 30 }),
    };
    let response = ResponseObject::ResponseApi(value.clone());
    assert_filtered_counts(&response);
    assert_eq!(reconcile_io_usage(&response), (70, 20, true));
    assert_eq!(response, ResponseObject::ResponseApi(value));
}

#[test]
fn mapping_response_token_names() {
    let mut value = usage();
    value.input_tokens = 100;
    value.output_tokens = 20;
    value.prompt_tokens = 0;
    value.completion_tokens = 0;
    value.cached_tokens = None;
    value.input_tokens_details = Some(TokenDetails { cached_tokens: 30 });
    let response = ResponseObject::Mapping(value.clone());
    assert_filtered_counts(&response);
    assert_eq!(reconcile_io_usage(&response), (70, 20, true));
    assert_eq!(response, ResponseObject::Mapping(value));
}

#[test]
fn mapping_completion_token_names() {
    let response = ResponseObject::Mapping(usage());
    assert_filtered_counts(&response);
}
