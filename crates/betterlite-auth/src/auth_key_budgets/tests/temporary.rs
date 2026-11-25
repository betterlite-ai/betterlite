use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use rstest::rstest;

use crate::auth_key_budgets::expiry::{
    MAX_TEMP_BUDGET_EXPIRY_BYTES, TempBudgetError, parse_temp_budget_expiry,
};
use crate::auth_key_budgets::key_budget::KeyBudgets;
use crate::auth_key_budgets::money::Money;
use crate::auth_key_budgets::temp::{
    TempBudgetUpdate, apply_temp_budget_increase, get_temp_budget_increase,
};

use super::{
    KeyBudgetError, ScriptedReader, admission, admit, dec, dec_text, effective_budgets, key, money,
    now,
};

fn temp_max(token: &str, expiry: &str) -> Option<Money> {
    let fields = format!(
        r#"{{"token":"{token}","max_budget":100,"metadata":{{"temp_budget_increase":30,"temp_budget_expiry":"{expiry}"}}}}"#
    );
    let auth = key(&fields);
    apply_temp_budget_increase(&auth, now()).unwrap().max_budget
}

#[rstest]
#[case::increase_only(r#"{"temp_budget_increase":50}"#)]
#[case::expiry_only(r#"{"temp_budget_expiry":"2027-01-01T00:00:00Z"}"#)]
#[case::empty("{}")]
fn both_fields_required(#[case] metadata: &str) {
    let fields = format!(r#"{{"token":"sk-temp","max_budget":100,"metadata":{metadata}}}"#);
    let auth = key(&fields);
    assert_eq!(get_temp_budget_increase(&auth, now()).unwrap(), None);
    assert_eq!(
        apply_temp_budget_increase(&auth, now()).unwrap(),
        TempBudgetUpdate {
            max_budget: Some(money(100)),
            applied: false,
        }
    );
}

#[rstest]
#[case::before_now("2026-01-15T12:30:00", 130_i64)]
#[case::after_now("2026-01-15T11:30:00", 100_i64)]
fn naive_expiry_utc(#[case] expiry: &str, #[case] expected: i64) {
    assert_eq!(
        temp_max("sk-naive", expiry),
        Some(money(expected)),
        "{expiry}"
    );
}

#[rstest]
#[case::plus_two("2026-01-15T14:30:00+02:00", 130_i64)]
#[case::minus_five_at("2026-01-15T07:00:00-05:00", 100_i64)]
#[case::minus_five_after("2026-01-15T07:00:01-05:00", 130_i64)]
#[case::zulu("2026-01-15T12:30:00Z", 130_i64)]
fn offset_expiry(#[case] expiry: &str, #[case] expected: i64) {
    assert_eq!(
        temp_max("sk-offset", expiry),
        Some(money(expected)),
        "{expiry}"
    );
}

#[test]
fn expiry_equal_inactive() {
    let auth = key(
        r#"{"token":"sk-equal-expiry","max_budget":100,"metadata":{"temp_budget_increase":30,"temp_budget_expiry":"2026-01-15T12:00:00Z"}}"#,
    );
    assert_eq!(get_temp_budget_increase(&auth, now()).unwrap(), None);
    assert_eq!(
        apply_temp_budget_increase(&auth, now()).unwrap(),
        TempBudgetUpdate {
            max_budget: Some(money(100)),
            applied: false,
        }
    );
}

#[rstest]
#[case::bad_date("\"not-a-date\"", "5", TempBudgetError::ExpiryInvalid)]
#[case::number_expiry("5", "5", TempBudgetError::ExpiryInvalid)]
#[case::bad_increase(
    "\"2027-01-01T00:00:00Z\"",
    "\"abc\"",
    TempBudgetError::IncreaseInvalid
)]
#[case::bool_increase("\"2027-01-01T00:00:00Z\"", "true", TempBudgetError::IncreaseInvalid)]
#[case::numeric_string_increase(
    "\"2027-01-01T00:00:00Z\"",
    "\"30\"",
    TempBudgetError::IncreaseInvalid
)]
fn invalid_expiry_propagates(
    #[case] expiry: &str,
    #[case] increase: &str,
    #[case] error: TempBudgetError,
) {
    let fields = format!(
        r#"{{"token":"sk-invalid","max_budget":100,"metadata":{{"temp_budget_increase":{increase},"temp_budget_expiry":{expiry}}}}}"#
    );
    let auth = key(&fields);
    assert_eq!(
        get_temp_budget_increase(&auth, now()).unwrap_err(),
        error,
        "{expiry} {increase}"
    );
    assert_eq!(apply_temp_budget_increase(&auth, now()).unwrap_err(), error);
    assert_eq!(effective_budgets(&auth, now()).unwrap_err(), error);

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
fn null_increase_keeps_original() {
    let auth = key(
        r#"{"token":"sk-null-increase","max_budget":100,"metadata":{"temp_budget_increase":null,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    assert_eq!(get_temp_budget_increase(&auth, now()).unwrap(), None);
    assert_eq!(
        apply_temp_budget_increase(&auth, now()).unwrap(),
        TempBudgetUpdate {
            max_budget: Some(money(100)),
            applied: false,
        }
    );
}

#[test]
fn no_max_skips_parse() {
    let auth = key(
        r#"{"token":"sk-no-max-temp","metadata":{"temp_budget_increase":10,"temp_budget_expiry":"garbage"}}"#,
    );
    assert_eq!(
        apply_temp_budget_increase(&auth, now()).unwrap(),
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

#[rstest]
#[case::zero("0")]
#[case::zero_float("0.0")]
#[case::negative_zero("-0")]
fn zero_increase_same_token(#[case] increase: &str) {
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
        apply_temp_budget_increase(&auth, now()).unwrap(),
        TempBudgetUpdate {
            max_budget: Some(money(100)),
            applied: false,
        }
    );
}

#[test]
fn negative_increase_applies() {
    let auth = key(
        r#"{"token":"sk-negative","max_budget":100,"metadata":{"temp_budget_increase":-30,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    assert_eq!(
        apply_temp_budget_increase(&auth, now()).unwrap(),
        TempBudgetUpdate {
            max_budget: Some(money(70)),
            applied: true,
        }
    );
    let below = key(
        r#"{"token":"sk-negative-more","max_budget":10,"metadata":{"temp_budget_increase":-30.5,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    assert_eq!(
        apply_temp_budget_increase(&below, now())
            .unwrap()
            .max_budget,
        Some(Money::Finite(dec_text("-20.5")))
    );
}

#[test]
fn increase_copies_token() {
    let auth = key(
        r#"{"token":"sk-copy","max_budget":100,"spend":3,"metadata":{"temp_budget_increase":30,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    let update = apply_temp_budget_increase(&auth, now()).unwrap();
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
        apply_temp_budget_increase(&auth, now()).unwrap().max_budget,
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

#[rstest]
#[case::date("20260116")]
#[case::concatenated("20260116T130000")]
#[case::minutes("20260116T1300")]
#[case::hours("20260116T13")]
#[case::dashed_date("2026-01-16T130000")]
#[case::offset("20260116T130000+0200")]
#[case::fraction("2026-01-16T130000.5")]
fn basic_iso_expiry_forms(#[case] expiry: &str) {
    let fields = format!(
        r#"{{"token":"sk-basic","max_budget":100,"metadata":{{"temp_budget_increase":10,"temp_budget_expiry":"{expiry}"}}}}"#
    );
    let auth = key(&fields);
    assert_eq!(
        apply_temp_budget_increase(&auth, now()).unwrap().max_budget,
        Some(money(110)),
        "{expiry}"
    );
    assert_eq!(
        parse_temp_budget_expiry("2026-01-16T13:00:00Z").unwrap(),
        super::instant("2026-01-16T13:00:00+00:00")
    );
}

#[tokio::test]
async fn increase_adds_exactly() {
    let mut auth = key(
        r#"{"token":"sk-tenth-increase","max_budget":0.1,"metadata":{"temp_budget_increase":0.2,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    let budgets = effective_budgets(&auth, now()).unwrap();
    assert_eq!(
        budgets.max_budget,
        Some(Money::Finite(dec_text("0.3"))),
        "0.1 + 0.2 is 0.3 exactly, where LiteLLM's doubles give 0.30000000000000004"
    );
    let reader = ScriptedReader::new([dec_text("0.3")]);
    let error = admit(&mut auth, &budgets, &admission(&reader, None))
        .await
        .unwrap_err();
    assert!(
        matches!(error, KeyBudgetError::Denied(_)),
        "a spend equal to the exact increased maximum is denied"
    );
}

#[test]
fn expiry_timestamp_forms() {
    assert_eq!(
        parse_temp_budget_expiry("2026-W03-4").unwrap_err(),
        TempBudgetError::ExpiryInvalid,
        "fromisoformat reads a week date; the shared speedate parser does not"
    );
    assert_eq!(
        parse_temp_budget_expiry("2026-01-15T12:30:00z").unwrap(),
        super::instant("2026-01-15T12:30:00+00:00"),
        "the shared parser reads a lowercase z where fromisoformat rejects it"
    );
}

#[rstest]
fn replays_the_auth_key_budgets_corpus(#[files("fuzz/corpus/auth_key_budgets/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = from_utf8(&bytes) else {
        return;
    };
    drop(parse_temp_budget_expiry(text));
}
