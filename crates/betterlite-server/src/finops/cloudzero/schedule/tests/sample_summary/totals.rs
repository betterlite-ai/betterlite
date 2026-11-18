use super::*;

#[test]
fn cloudzero_schedule() {
    let rows = [
        row(&[
            ("spend", decimal("1.25")),
            ("prompt_tokens", integer(3)),
            ("completion_tokens", integer(1)),
        ]),
        row(&[
            ("spend", decimal("2.75")),
            ("prompt_tokens", integer(0)),
            ("completion_tokens", integer(2)),
        ]),
        row(&[]),
    ];
    let summary = sample_summary(&rows).expect("a summary");
    assert_eq!(summary.total_records, 3);
    assert_eq!(summary.total_cost, Decimal::new(4, 0));
    assert_eq!(summary.total_tokens, 6);
    assert_eq!(summary.unique_accounts, 0);
    assert_eq!(summary.unique_services, 0);
}
