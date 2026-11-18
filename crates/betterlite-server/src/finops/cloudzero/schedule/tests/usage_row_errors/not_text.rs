use super::*;

#[test]
fn cloudzero_schedule() {
    let not_text = usage_record(&row(&[("model", integer(1))])).expect_err("a number is not text");
    assert_eq!(
        not_text.to_string(),
        "the usage row field 'model' is not text"
    );
    assert!(matches!(
        not_text,
        UsageRowError::NotText { field: "model" }
    ));

    let wrong_bool =
        usage_record(&row(&[("date", JsonData::Bool(true))])).expect_err("a bool is not text");
    assert!(matches!(
        wrong_bool,
        UsageRowError::NotText { field: "date" }
    ));
}
