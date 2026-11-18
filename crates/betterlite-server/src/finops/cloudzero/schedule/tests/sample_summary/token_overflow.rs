use super::*;

#[test]
fn cloudzero_schedule() {
    let token_overflow = [
        row(&[
            ("prompt_tokens", integer(i64::MAX)),
            ("completion_tokens", integer(0)),
        ]),
        row(&[
            ("prompt_tokens", integer(1)),
            ("completion_tokens", integer(0)),
        ]),
    ];
    let error = sample_summary(&token_overflow).expect_err("a token overflow");
    assert_eq!(
        error.to_string(),
        "the dry run total for total_tokens overflowed"
    );
}
