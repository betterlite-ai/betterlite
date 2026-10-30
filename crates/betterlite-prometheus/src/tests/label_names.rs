#![cfg(test)]
use betterlite_prometheus::label_names_catalog::{
    EXCEPTION_LABELS, RATE_LIMIT_CATEGORY, RATE_LIMIT_TYPE, REQUESTED_MODEL, STATUS_CODE,
    UserApiKeyLabelName, ZERO_COST_REASON_LABEL,
};

#[test]
fn exposes_domains() {
    let expected = include_str!("fixtures/labels.txt")
        .lines()
        .collect::<Vec<_>>();
    assert_eq!(
        UserApiKeyLabelName::ALL.map(UserApiKeyLabelName::as_str),
        expected.as_slice()
    );
    assert_eq!(UserApiKeyLabelName::ALL.len(), 33);
    assert_eq!(EXCEPTION_LABELS, ["exception_status", "exception_class"]);
    assert_eq!(
        [
            REQUESTED_MODEL,
            RATE_LIMIT_CATEGORY,
            RATE_LIMIT_TYPE,
            STATUS_CODE,
            ZERO_COST_REASON_LABEL
        ],
        [
            "requested_model",
            "rate_limit_category",
            "rate_limit_type",
            "status_code",
            "reason"
        ]
    );
}
