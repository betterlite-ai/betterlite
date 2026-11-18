use super::*;

#[test]
fn cloudzero_schedule() {
    let text_spend = [row(&[("spend", text("free"))])];
    let error = sample_summary(&text_spend).expect_err("text is not a decimal");
    assert_eq!(error.to_string(), "the usage rows could not be read");
    assert!(matches!(
        error,
        DryRunError::Usage(UsageRowError::NotDecimal { field: "spend" })
    ));
}
