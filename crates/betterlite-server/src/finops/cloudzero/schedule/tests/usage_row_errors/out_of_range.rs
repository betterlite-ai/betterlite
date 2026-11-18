use super::*;

#[test]
fn cloudzero_schedule() {
    let out_of_range = usage_record(&row(&[("spend", decimal("1e400"))]))
        .expect_err("an oversized number is refused");
    assert_eq!(
        out_of_range.to_string(),
        "the usage row field 'spend' is not a valid decimal number"
    );
    assert!(matches!(
        out_of_range,
        UsageRowError::Decimal { field: "spend", .. }
    ));
}
