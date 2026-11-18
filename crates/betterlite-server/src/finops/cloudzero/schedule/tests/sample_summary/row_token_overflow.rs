use super::*;

#[test]
fn cloudzero_schedule() {
    let row_overflow = [row(&[
        ("prompt_tokens", integer(i64::MAX)),
        ("completion_tokens", integer(1)),
    ])];
    let error = sample_summary(&row_overflow).expect_err("a row token overflow");
    assert_eq!(
        error.to_string(),
        "the dry run total for total_tokens overflowed"
    );
}
