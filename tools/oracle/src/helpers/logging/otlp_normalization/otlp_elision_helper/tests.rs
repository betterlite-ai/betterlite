#![cfg(test)]

use betterlite_oracle::case::parse_helper_case;

#[test]
fn otlp_elision_helper() {
    parse_helper_case(include_str!(
        "../../../../../fuzz/corpus/helper_case/otlp_normalization-elision.yaml"
    ))
    .unwrap();
}
