#![cfg(test)]

use betterlite_oracle::case::parse_helper_case;

#[test]
fn otlp_response_encoding_helper() {
    parse_helper_case(include_str!(
        "../../../../../fuzz/corpus/helper_case/otlp_normalization-response.yaml"
    ))
    .unwrap();
}
