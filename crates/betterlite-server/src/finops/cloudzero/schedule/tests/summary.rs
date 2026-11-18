#![cfg(test)]

use betterlite_logging::finops::cloudzero::records::{CbfRecord, CbfValue};
use rust_decimal::Decimal;

use crate::finops::cloudzero::schedule::dry_run::cbf_summary;

fn entry(cost: Decimal, tokens: i64, account: &str, service: &str) -> CbfRecord {
    CbfRecord::from([
        ("cost/cost".to_owned(), CbfValue::Money(cost)),
        ("usage/amount".to_owned(), CbfValue::Integer(tokens)),
        (
            "resource/account".to_owned(),
            CbfValue::Text(account.to_owned()),
        ),
        (
            "resource/service".to_owned(),
            CbfValue::Text(service.to_owned()),
        ),
    ])
}

#[test]
fn cloudzero_schedule() {
    let records = [
        entry(Decimal::new(15, 1), 10, "account-a", "service-a"),
        entry(Decimal::new(25, 1), 20, "account-b", "service-b"),
        entry(Decimal::new(5, 1), 5, "", ""),
        CbfRecord::new(),
    ];
    let summary = cbf_summary(&records).expect("a summary");
    assert_eq!(summary.total_records, 4);
    assert_eq!(summary.total_cost, Decimal::new(45, 1));
    assert_eq!(summary.total_tokens, 35);
    assert_eq!(summary.unique_accounts, 2);
    assert_eq!(summary.unique_services, 2);

    let cost_overflow = [
        entry(Decimal::MAX, 0, "", ""),
        entry(Decimal::ONE, 0, "", ""),
    ];
    let error = cbf_summary(&cost_overflow).expect_err("a cost overflow");
    assert_eq!(
        error.to_string(),
        "the dry run total for total_cost overflowed"
    );

    let token_overflow = [
        entry(Decimal::ZERO, i64::MAX, "", ""),
        entry(Decimal::ZERO, 1, "", ""),
    ];
    let token_error = cbf_summary(&token_overflow).expect_err("a token overflow");
    assert_eq!(
        token_error.to_string(),
        "the dry run total for total_tokens overflowed"
    );
}
