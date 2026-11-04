#![cfg(test)]

use super::{MAX_SPLIT_OUTPUT_BYTES, SplitError, SplitOutput};

#[test]
fn split_output_cap_accepts_exactly_the_byte_limit() {
    let mut output = SplitOutput {
        chunks: Vec::new(),
        total_bytes: MAX_SPLIT_OUTPUT_BYTES - 2,
    };
    output
        .push("é".to_owned())
        .expect("two remaining bytes fit");
    assert_eq!(output.total_bytes, MAX_SPLIT_OUTPUT_BYTES);
    assert_eq!(output.chunks, ["é"]);
    assert_eq!(
        output.push("x".to_owned()),
        Err(SplitError::OutputTooLarge {
            limit: MAX_SPLIT_OUTPUT_BYTES
        })
    );
    assert_eq!(output.chunks, ["é"]);
}
