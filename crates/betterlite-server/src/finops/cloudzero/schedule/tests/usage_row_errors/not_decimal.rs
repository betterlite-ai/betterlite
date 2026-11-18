use super::*;

#[test]
fn cloudzero_schedule() {
    let not_decimal =
        usage_record(&row(&[("spend", text("1.25"))])).expect_err("text is not a decimal");
    assert_eq!(
        not_decimal.to_string(),
        "the usage row field 'spend' is not a decimal number"
    );
    assert!(matches!(
        not_decimal,
        UsageRowError::NotDecimal { field: "spend" }
    ));
}
