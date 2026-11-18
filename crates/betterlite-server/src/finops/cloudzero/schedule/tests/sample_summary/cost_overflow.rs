use super::*;

#[test]
fn cloudzero_schedule() {
    let cost_overflow = [
        row(&[("spend", decimal("79228162514264337593543950335"))]),
        row(&[("spend", decimal("1"))]),
    ];
    let error = sample_summary(&cost_overflow).expect_err("a cost overflow");
    assert_eq!(
        error.to_string(),
        "the dry run total for total_cost overflowed"
    );
}
