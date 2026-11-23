use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::auth_key_budgets::expiry::{
    MAX_TEMP_BUDGET_EXPIRY_BYTES, TempBudgetError, parse_temp_budget_expiry,
};
use crate::auth_key_budgets::key_budget::KeyBudgets;
use crate::auth_key_budgets::money::Money;
use crate::auth_key_budgets::temp::{
    TempBudgetUpdate, get_temp_budget_increase, update_key_budget_with_temp_budget_increase,
};

use super::{dec, effective_budgets, key, money, now};

#[test]
fn both_fields_required() {
    for metadata in [
        r#"{"temp_budget_increase":50}"#,
        r#"{"temp_budget_expiry":"2027-01-01T00:00:00Z"}"#,
        r#"{}"#,
    ] {
        let fields = format!(r#"{{"token":"sk-temp","max_budget":100,"metadata":{metadata}}}"#);
        let auth = key(&fields);
        assert_eq!(get_temp_budget_increase(&auth, now()).unwrap(), None);
        assert_eq!(
            update_key_budget_with_temp_budget_increase(&auth, now()).unwrap(),
            TempBudgetUpdate {
                max_budget: Some(money(100)),
                applied: false,
            }
        );
    }
}

#[test]
fn naive_expiry_utc() {
    for (expiry, expected) in [
        ("2026-01-15T12:30:00", Some(money(130))),
        ("2026-01-15T11:30:00", Some(money(100))),
    ] {
        let fields = format!(
            r#"{{"token":"sk-naive","max_budget":100,"metadata":{{"temp_budget_increase":30,"temp_budget_expiry":"{expiry}"}}}}"#
        );
        let auth = key(&fields);
        assert_eq!(
            update_key_budget_with_temp_budget_increase(&auth, now())
                .unwrap()
                .max_budget,
            expected,
            "{expiry}"
        );
    }
}

#[test]
fn offset_expiry() {
    for (expiry, expected) in [
        ("2026-01-15T14:30:00+02:00", money(130)),
        ("2026-01-15T07:00:00-05:00", money(100)),
        ("2026-01-15T07:00:01-05:00", money(130)),
        ("2026-01-15T12:30:00Z", money(130)),
    ] {
        let fields = format!(
            r#"{{"token":"sk-offset","max_budget":100,"metadata":{{"temp_budget_increase":30,"temp_budget_expiry":"{expiry}"}}}}"#
        );
        let auth = key(&fields);
        assert_eq!(
            update_key_budget_with_temp_budget_increase(&auth, now())
                .unwrap()
                .max_budget,
            Some(expected),
            "{expiry}"
        );
    }
}

#[test]
fn expiry_equal_inactive() {
    let auth = key(
        r#"{"token":"sk-equal-expiry","max_budget":100,"metadata":{"temp_budget_increase":30,"temp_budget_expiry":"2026-01-15T12:00:00Z"}}"#,
    );
    assert_eq!(get_temp_budget_increase(&auth, now()).unwrap(), None);
    assert_eq!(
        update_key_budget_with_temp_budget_increase(&auth, now()).unwrap(),
        TempBudgetUpdate {
            max_budget: Some(money(100)),
            applied: false,
        }
    );
}

#[test]
fn invalid_expiry_propagates() {
    for (expiry, increase, error) in [
        ("\"not-a-date\"", "5", TempBudgetError::ExpiryInvalid),
        ("5", "5", TempBudgetError::ExpiryInvalid),
        (
            "\"2027-01-01T00:00:00Z\"",
            "\"abc\"",
            TempBudgetError::IncreaseInvalid,
        ),
        (
            "\"2027-01-01T00:00:00Z\"",
            "true",
            TempBudgetError::IncreaseInvalid,
        ),
    ] {
        let fields = format!(
            r#"{{"token":"sk-invalid","max_budget":100,"metadata":{{"temp_budget_increase":{increase},"temp_budget_expiry":{expiry}}}}}"#
        );
        let auth = key(&fields);
        assert_eq!(
            get_temp_budget_increase(&auth, now()).unwrap_err(),
            error,
            "{expiry} {increase}"
        );
        assert_eq!(
            update_key_budget_with_temp_budget_increase(&auth, now()).unwrap_err(),
            error
        );
        assert_eq!(effective_budgets(&auth, now()).unwrap_err(), error);
    }

    let expired = key(
        r#"{"token":"sk-expired-bad-increase","max_budget":100,"metadata":{"temp_budget_increase":"abc","temp_budget_expiry":"2026-01-15T11:00:00Z"}}"#,
    );
    assert_eq!(
        get_temp_budget_increase(&expired, now()).unwrap(),
        None,
        "an expired date is read before the increase"
    );
}

#[test]
fn no_max_skips_parse() {
    let auth = key(
        r#"{"token":"sk-no-max-temp","metadata":{"temp_budget_increase":10,"temp_budget_expiry":"garbage"}}"#,
    );
    assert_eq!(
        update_key_budget_with_temp_budget_increase(&auth, now()).unwrap(),
        TempBudgetUpdate {
            max_budget: None,
            applied: false,
        }
    );
    assert_eq!(
        get_temp_budget_increase(&auth, now()).unwrap_err(),
        TempBudgetError::ExpiryInvalid
    );
    assert_eq!(effective_budgets(&auth, now()).unwrap().max_budget, None);
}

#[test]
fn zero_increase_same_token() {
    for increase in ["0", "0.0", "-0"] {
        let fields = format!(
            r#"{{"token":"sk-zero-increase","max_budget":100,"metadata":{{"temp_budget_increase":{increase},"temp_budget_expiry":"2027-01-01T00:00:00Z"}}}}"#
        );
        let auth = key(&fields);
        assert_eq!(
            get_temp_budget_increase(&auth, now()).unwrap(),
            Some(money(0)),
            "{increase}"
        );
        assert_eq!(
            update_key_budget_with_temp_budget_increase(&auth, now()).unwrap(),
            TempBudgetUpdate {
                max_budget: Some(money(100)),
                applied: false,
            }
        );
    }
}

#[test]
fn negative_increase_applies() {
    let auth = key(
        r#"{"token":"sk-negative","max_budget":100,"metadata":{"temp_budget_increase":-30,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    assert_eq!(
        update_key_budget_with_temp_budget_increase(&auth, now()).unwrap(),
        TempBudgetUpdate {
            max_budget: Some(money(70)),
            applied: true,
        }
    );
    let below = key(
        r#"{"token":"sk-negative-more","max_budget":10,"metadata":{"temp_budget_increase":-30.5,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    assert_eq!(
        update_key_budget_with_temp_budget_increase(&below, now())
            .unwrap()
            .max_budget,
        Some(Money::Finite(super::dec_text("-20.5")))
    );
}

#[test]
fn increase_copies_token() {
    let auth = key(
        r#"{"token":"sk-copy","max_budget":100,"spend":3,"metadata":{"temp_budget_increase":30,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    let update = update_key_budget_with_temp_budget_increase(&auth, now()).unwrap();
    assert!(update.applied);
    assert_eq!(
        effective_budgets(&auth, now()).unwrap(),
        KeyBudgets {
            max_budget: Some(money(130)),
            soft_budget: None,
        }
    );
    assert_eq!(auth.view.token.max_budget, Some(dec(100)));
    assert_eq!(auth.view.token.spend, dec(3));
    assert_eq!(
        update_key_budget_with_temp_budget_increase(&auth, now())
            .unwrap()
            .max_budget,
        Some(money(130)),
        "the update is computed again without mutating the token"
    );
}

#[test]
fn expiry_input_limit() {
    let at_limit = "1".repeat(MAX_TEMP_BUDGET_EXPIRY_BYTES);
    assert_eq!(
        parse_temp_budget_expiry(&at_limit).unwrap_err(),
        TempBudgetError::ExpiryInvalid
    );
    let over_limit = "1".repeat(MAX_TEMP_BUDGET_EXPIRY_BYTES + 1);
    assert_eq!(
        parse_temp_budget_expiry(&over_limit).unwrap_err(),
        TempBudgetError::ExpiryTooLong
    );
}

#[test]
fn basic_iso_expiry_forms() {
    for expiry in [
        "20260116",
        "20260116T130000",
        "20260116T1300",
        "20260116T13",
        "2026-01-16T130000",
        "20260116T130000+0200",
        "2026-01-16T130000.5",
    ] {
        let fields = format!(
            r#"{{"token":"sk-basic","max_budget":100,"metadata":{{"temp_budget_increase":10,"temp_budget_expiry":"{expiry}"}}}}"#
        );
        let auth = key(&fields);
        assert_eq!(
            update_key_budget_with_temp_budget_increase(&auth, now())
                .unwrap()
                .max_budget,
            Some(money(110)),
            "{expiry}"
        );
    }
    assert_eq!(
        parse_temp_budget_expiry("2026-01-16T13:00:00Z").unwrap(),
        super::instant("2026-01-16T13:00:00+00:00")
    );
}

#[rstest]
fn replays_the_auth_key_budgets_corpus(#[files("fuzz/corpus/auth_key_budgets/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return;
    };
    drop(parse_temp_budget_expiry(text));
}
