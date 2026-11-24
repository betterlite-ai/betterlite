use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use crate::key_fields::values::Amount as KeyAmount;
use crate::key_limit_policy::error::{KeyLimitError, Rejection};
use crate::key_limit_policy::request::{KeyLimitRequest, NumericValue};
use crate::key_limit_policy::upperbound::{check_finite, enforce_upperbound};
use crate::management_guards::amounts::Amount as GuardAmount;
use crate::team_parent_limits::error::RejectionDetail;

use super::support::{generate, params as configured, rejected, time, update};

#[test]
fn upperbound_key_param() {
    let ceiling = configured(
        r#"{"max_budget": 10, "max_parallel_requests": 4, "tpm_limit": 100, "rpm_limit": 5, "duration": "1d", "budget_duration": "30d"}"#,
    );
    let request = generate(r#"{"max_budget": 3, "rpm_limit": 2, "duration": "12h"}"#);
    let database = TimeZoneDatabase::from_env();
    let values = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&ceiling),
        true,
        &time(&database),
    )
    .unwrap();
    assert_eq!(values.max_budget, Some(KeyAmount(Decimal::from(3))));
    assert_eq!(values.max_parallel_requests, Some(4));
    assert_eq!(values.tpm_limit, Some(100));
    assert_eq!(values.rpm_limit, Some(2));
    assert_eq!(values.duration.as_deref(), Some("12h"));
    assert_eq!(values.budget_duration.as_deref(), Some("30d"));
}

#[test]
fn upperbound_fill_generation() {
    let ceiling = configured(
        r#"{"max_budget": 10, "max_parallel_requests": 4, "tpm_limit": 100, "rpm_limit": 5, "duration": "1d", "budget_duration": "30d"}"#,
    );
    let request = generate(r#"{"key_alias": "alias-1"}"#);
    let database = TimeZoneDatabase::from_env();
    let values = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&ceiling),
        true,
        &time(&database),
    )
    .unwrap();
    assert_eq!(values.max_budget, Some(KeyAmount(Decimal::from(10))));
    assert_eq!(values.max_parallel_requests, Some(4));
    assert_eq!(values.tpm_limit, Some(100));
    assert_eq!(values.rpm_limit, Some(5));
    assert_eq!(values.duration.as_deref(), Some("1d"));
    assert_eq!(values.budget_duration.as_deref(), Some("30d"));
}

#[test]
fn upperbound_update_omits() {
    let ceiling = configured(r#"{"max_budget": 10, "tpm_limit": 100, "duration": "1d"}"#);
    let request = update(r#"{"key": "sk-abc", "max_budget": 2}"#);
    let database = TimeZoneDatabase::from_env();
    let values = enforce_upperbound(
        &KeyLimitRequest::Update(&request),
        Some(&ceiling),
        false,
        &time(&database),
    )
    .unwrap();
    assert_eq!(values.max_budget, Some(KeyAmount(Decimal::from(2))));
    assert_eq!(values.tpm_limit, None);
    assert_eq!(values.duration, None);
    assert_eq!(values.budget_duration, None);
}

#[test]
fn finite_without_config() {
    let request = generate(r#"{"max_budget": 5}"#);
    let database = TimeZoneDatabase::from_env();
    let values = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        None,
        true,
        &time(&database),
    )
    .unwrap();
    assert_eq!(values.max_budget, Some(KeyAmount(Decimal::from(5))));
    let values = [
        NumericValue {
            field: "max_budget",
            value: Some(GuardAmount::Finite(Decimal::from(5))),
        },
        NumericValue {
            field: "soft_budget",
            value: Some(GuardAmount::Nan),
        },
    ];
    let rejection = check_finite(&values).unwrap_err();
    assert_eq!(
        rejection.to_string(),
        "soft_budget must be a finite number. Received: nan"
    );
    assert_eq!(
        rejection.detail(),
        RejectionDetail::Error {
            error: "soft_budget must be a finite number. Received: nan".to_owned(),
        }
    );
}

#[test]
fn non_finite_each_direction() {
    let cases = [
        (GuardAmount::PositiveInfinity, "max_budget", "inf"),
        (GuardAmount::NegativeInfinity, "tpd_limit", "-inf"),
        (GuardAmount::Nan, "rpm_limit", "nan"),
    ];
    for (amount, field, text) in cases {
        let values = [NumericValue {
            field,
            value: Some(amount),
        }];
        let rejection = check_finite(&values).unwrap_err();
        assert_eq!(
            rejection.to_string(),
            format!("{field} must be a finite number. Received: {text}")
        );
    }
    let missing = [NumericValue {
        field: "max_budget",
        value: None,
    }];
    check_finite(&missing).unwrap();
}

#[test]
fn finite_order_follows_the_request_fields() {
    let values = [
        NumericValue {
            field: "max_budget",
            value: Some(GuardAmount::Nan),
        },
        NumericValue {
            field: "tpm_limit",
            value: Some(GuardAmount::PositiveInfinity),
        },
    ];
    let rejection = check_finite(&values).unwrap_err();
    assert!(matches!(
        rejection,
        Rejection::NonFinite {
            field: "max_budget",
            ..
        }
    ));
}

#[test]
fn duration_limit() {
    let ceiling = configured(r#"{"duration": "1d"}"#);
    let database = TimeZoneDatabase::from_env();
    let request = generate(r#"{"duration": "30d"}"#);
    let result = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&ceiling),
        true,
        &time(&database),
    );
    assert_eq!(
        rejected(result).to_string(),
        "duration is over max limit set in config - user_value=30d; max_value=1d"
    );
    let infinite = generate(r#"{"duration": "-1"}"#);
    let result = enforce_upperbound(
        &KeyLimitRequest::Generate(&infinite),
        Some(&ceiling),
        true,
        &time(&database),
    );
    assert_eq!(
        rejected(result).to_string(),
        "duration is over max limit set in config - user_value=-1; max_value=1d"
    );
    let within = generate(r#"{"duration": "12h"}"#);
    let values = enforce_upperbound(
        &KeyLimitRequest::Generate(&within),
        Some(&ceiling),
        true,
        &time(&database),
    )
    .unwrap();
    assert_eq!(values.duration.as_deref(), Some("12h"));
}

#[test]
fn budget_duration_limit() {
    let ceiling = configured(r#"{"budget_duration": "1d"}"#);
    let database = TimeZoneDatabase::from_env();
    let request = generate(r#"{"budget_duration": "2d"}"#);
    let result = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&ceiling),
        true,
        &time(&database),
    );
    assert_eq!(
        rejected(result).to_string(),
        "budget_duration is over max limit set in config - user_value=2d; max_value=1d"
    );
    let equal = generate(r#"{"budget_duration": "1d"}"#);
    let values = enforce_upperbound(
        &KeyLimitRequest::Generate(&equal),
        Some(&ceiling),
        true,
        &time(&database),
    )
    .unwrap();
    assert_eq!(values.budget_duration.as_deref(), Some("1d"));
}

#[test]
fn invalid_duration() {
    let database = TimeZoneDatabase::from_env();
    let bad_ceiling = configured(r#"{"duration": "abc"}"#);
    let request = generate(r#"{"duration": "1d"}"#);
    let result = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&bad_ceiling),
        true,
        &time(&database),
    );
    match result {
        Err(KeyLimitError::Duration { field, .. }) => assert_eq!(field, "duration"),
        other => panic!("expected the configured duration to fail, got {other:?}"),
    }
    let ceiling = configured(r#"{"duration": "1d"}"#);
    let bad_request = generate(r#"{"duration": "soon"}"#);
    let result = enforce_upperbound(
        &KeyLimitRequest::Generate(&bad_request),
        Some(&ceiling),
        true,
        &time(&database),
    );
    match result {
        Err(KeyLimitError::Duration { field, .. }) => assert_eq!(field, "duration"),
        other => panic!("expected the caller duration to fail, got {other:?}"),
    }
}

#[test]
fn equal_ceiling() {
    let ceiling = configured(
        r#"{"max_budget": 10, "max_parallel_requests": 4, "tpm_limit": 100, "rpm_limit": 5}"#,
    );
    let request = generate(
        r#"{"max_budget": 10, "max_parallel_requests": 4, "tpm_limit": 100, "rpm_limit": 5}"#,
    );
    let database = TimeZoneDatabase::from_env();
    let values = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&ceiling),
        true,
        &time(&database),
    )
    .unwrap();
    assert_eq!(values.max_budget, Some(KeyAmount(Decimal::from(10))));
    assert_eq!(values.rpm_limit, Some(5));
}

#[test]
fn unsupported_ceiling_field() {
    let ceiling = configured(r#"{"max_budget": 10}"#);
    let request = generate(r#"{"soft_budget": 1000, "tpd_limit": 1000}"#);
    let database = TimeZoneDatabase::from_env();
    let values = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&ceiling),
        true,
        &time(&database),
    )
    .unwrap();
    assert_eq!(values.max_budget, Some(KeyAmount(Decimal::from(10))));
    assert_eq!(values.tpm_limit, None);
}

#[test]
fn negative_and_zero_ceiling() {
    let ceiling = configured(r#"{"max_budget": 0, "rpm_limit": -1}"#);
    let database = TimeZoneDatabase::from_env();
    let request = generate(r#"{"max_budget": 0, "rpm_limit": -1}"#);
    enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&ceiling),
        true,
        &time(&database),
    )
    .unwrap();
    let over_budget = generate(r#"{"max_budget": 1}"#);
    let result = enforce_upperbound(
        &KeyLimitRequest::Generate(&over_budget),
        Some(&ceiling),
        true,
        &time(&database),
    );
    assert_eq!(
        rejected(result).to_string(),
        "max_budget is over max limit set in config - user_value=1; max_value=0"
    );
    let over_rpm = generate(r#"{"rpm_limit": 0}"#);
    let result = enforce_upperbound(
        &KeyLimitRequest::Generate(&over_rpm),
        Some(&ceiling),
        true,
        &time(&database),
    );
    assert_eq!(
        rejected(result).to_string(),
        "rpm_limit is over max limit set in config - user_value=0; max_value=-1"
    );
}

#[test]
fn failure_order_is_configuration_order() {
    let ceiling = configured(
        r#"{"duration": "1d", "max_budget": 10, "max_parallel_requests": 4, "tpm_limit": 100, "rpm_limit": 5}"#,
    );
    let request = generate(
        r#"{"duration": "2d", "max_budget": 20, "max_parallel_requests": 8, "tpm_limit": 200, "rpm_limit": 10}"#,
    );
    let database = TimeZoneDatabase::from_env();
    let result = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&ceiling),
        true,
        &time(&database),
    );
    assert_eq!(
        rejected(result).to_string(),
        "duration is over max limit set in config - user_value=2d; max_value=1d"
    );
    let ceiling = configured(r#"{"max_budget": 10, "max_parallel_requests": 4}"#);
    let request = generate(r#"{"max_parallel_requests": 8, "max_budget": 20}"#);
    let result = enforce_upperbound(
        &KeyLimitRequest::Generate(&request),
        Some(&ceiling),
        true,
        &time(&database),
    );
    assert_eq!(
        rejected(result).to_string(),
        "max_budget is over max limit set in config - user_value=20; max_value=10"
    );
}
