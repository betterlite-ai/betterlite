use super::*;

#[test]
fn record_types() {
    let spend = ROISpendRecord {
        date: "2026-01-01".to_owned(),
        user_id: "user".to_owned(),
        email: "user@example.com".to_owned(),
        spend: Decimal::new(125, 2),
        requests: 1,
    };
    assert_eq!(spend.spend, Decimal::new(125, 2));
    assert_eq!(spend.requests, 1);
}
