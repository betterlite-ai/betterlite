#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::Session;

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn parse(body: &str) -> Option<Session> {
    Session::from_json(body.as_bytes())
}

fn with(extra: &str) -> String {
    format!(r#"{{"router_name":"auto","last_model":"gpt-5","spend":2,{extra}}}"#)
}

#[test]
fn a_minimal_payload_is_a_session_with_every_option_unset() {
    let session = parse(r#"{"router_name":"auto","last_model":"gpt-5","spend":0.5}"#).unwrap();

    assert_eq!(
        session,
        Session {
            router_name: "auto".to_owned(),
            last_model: "gpt-5".to_owned(),
            spend: dec("0.5"),
            baseline_spend: None,
            baseline_model: None,
            turns: None,
            savings_estimated_turns: None,
            savings_estimated_actual_spend: None,
        }
    );
}

#[test]
fn a_full_payload_fills_every_field() {
    let body = r#"{"router_name":"auto","last_model":"gpt-5","spend":1.25,
        "savings_estimated_baseline_spend":4.5,"baseline_model":"claude-opus-4",
        "turns":9,"savings_estimated_turns":3,"savings_estimated_actual_spend":0.75}"#;

    assert_eq!(
        parse(body).unwrap(),
        Session {
            router_name: "auto".to_owned(),
            last_model: "gpt-5".to_owned(),
            spend: dec("1.25"),
            baseline_spend: Some(dec("4.5")),
            baseline_model: Some("claude-opus-4".to_owned()),
            turns: Some(9),
            savings_estimated_turns: Some(3),
            savings_estimated_actual_spend: Some(dec("0.75")),
        }
    );
}

#[rstest]
#[case::empty_body("")]
#[case::not_json("nope")]
#[case::array("[1]")]
#[case::string(r#""text""#)]
#[case::empty_object("{}")]
#[case::missing_router_name(r#"{"last_model":"m","spend":1}"#)]
#[case::empty_router_name(r#"{"router_name":"","last_model":"m","spend":1}"#)]
#[case::router_name_that_prints_as_nothing(
    r#"{"router_name":"\u001b","last_model":"m","spend":1}"#
)]
#[case::numeric_router_name(r#"{"router_name":7,"last_model":"m","spend":1}"#)]
#[case::null_router_name(r#"{"router_name":null,"last_model":"m","spend":1}"#)]
#[case::missing_last_model(r#"{"router_name":"r","spend":1}"#)]
#[case::empty_last_model(r#"{"router_name":"r","last_model":"","spend":1}"#)]
#[case::last_model_that_prints_as_nothing(r#"{"router_name":"r","last_model":"\u0007","spend":1}"#)]
#[case::missing_spend(r#"{"router_name":"r","last_model":"m"}"#)]
#[case::null_spend(r#"{"router_name":"r","last_model":"m","spend":null}"#)]
#[case::string_spend(r#"{"router_name":"r","last_model":"m","spend":"1.5"}"#)]
#[case::boolean_spend(r#"{"router_name":"r","last_model":"m","spend":true}"#)]
#[case::object_spend(r#"{"router_name":"r","last_model":"m","spend":{}}"#)]
#[case::list_spend(r#"{"router_name":"r","last_model":"m","spend":[1]}"#)]
#[case::spend_too_large_to_be_finite(r#"{"router_name":"r","last_model":"m","spend":1e999}"#)]
fn an_invalid_payload_is_no_session(#[case] body: &str) {
    assert_eq!(parse(body), None);
}

#[rstest]
#[case::string_baseline(r#""3""#)]
#[case::boolean_baseline("true")]
#[case::object_baseline("{}")]
#[case::list_baseline("[3]")]
fn a_baseline_that_is_not_a_number_rejects_the_payload(#[case] value: &str) {
    assert_eq!(parse(&with(&format!(r#""baseline_spend":{value}"#))), None);
    assert_eq!(
        parse(&with(&format!(
            r#""savings_estimated_baseline_spend":{value}"#
        ))),
        None
    );
}

#[test]
fn a_bad_estimated_baseline_wins_over_a_good_plain_one() {
    let body = with(r#""savings_estimated_baseline_spend":"x","baseline_spend":3"#);

    assert_eq!(parse(&body), None);
}

#[rstest]
#[case::integer("2", "2")]
#[case::negative("-0.5", "-0.5")]
#[case::keeps_every_digit("0.1234567890123456789", "0.1234567890123456789")]
#[case::keeps_trailing_zeros("1.50", "1.50")]
#[case::exponent("1e2", "100")]
#[case::negative_exponent("2.5E-3", "0.0025")]
#[case::zero("0", "0")]
fn money_is_read_as_a_decimal(#[case] literal: &str, #[case] expected: &str) {
    let body = format!(r#"{{"router_name":"r","last_model":"m","spend":{literal}}}"#);

    assert_eq!(parse(&body).unwrap().spend, dec(expected));
    assert_eq!(parse(&body).unwrap().spend.to_string(), expected);
}

#[rstest]
#[case::estimated_key_alone(r#""savings_estimated_baseline_spend":4"#, Some("4"))]
#[case::plain_key_alone(r#""baseline_spend":3"#, Some("3"))]
#[case::estimated_key_wins(
    r#""savings_estimated_baseline_spend":4,"baseline_spend":3"#,
    Some("4")
)]
#[case::estimated_key_wins_in_either_order(
    r#""baseline_spend":3,"savings_estimated_baseline_spend":4"#,
    Some("4")
)]
#[case::null_estimated_key_hides_the_plain_one(
    r#""savings_estimated_baseline_spend":null,"baseline_spend":3"#,
    None
)]
#[case::null_plain_key(r#""baseline_spend":null"#, None)]
#[case::both_null(
    r#""savings_estimated_baseline_spend":null,"baseline_spend":null"#,
    None
)]
#[case::neither(r#""turns":1"#, None)]
#[case::zero_baseline_is_kept(r#""baseline_spend":0"#, Some("0"))]
fn the_estimated_baseline_key_wins_whenever_it_is_present(
    #[case] extra: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        parse(&with(extra)).unwrap().baseline_spend,
        expected.map(dec)
    );
}

#[rstest]
#[case::name(r#""baseline_model":"claude-opus-4""#, Some("claude-opus-4"))]
#[case::empty(r#""baseline_model":"""#, None)]
#[case::prints_as_nothing(r#""baseline_model":"\u001b""#, None)]
#[case::stripped(r#""baseline_model":"claude\u001b-4""#, Some("claude-4"))]
#[case::not_a_string(r#""baseline_model":7"#, None)]
#[case::null(r#""baseline_model":null"#, None)]
#[case::absent(r#""turns":1"#, None)]
fn the_baseline_model_is_a_printable_label_or_nothing(
    #[case] extra: &str,
    #[case] expected: Option<&str>,
) {
    let session = parse(&with(extra)).unwrap();

    assert_eq!(session.baseline_model.as_deref(), expected);
}

#[test]
fn router_and_model_names_are_stripped_of_control_characters() {
    let session =
        parse(r#"{"router_name":"au\u001bto","last_model":"gp\u0007t","spend":1}"#).unwrap();

    assert_eq!(session.router_name, "auto");
    assert_eq!(session.last_model, "gpt");
}

#[rstest]
#[case::count(r#""turns":3"#, Some(3))]
#[case::zero(r#""turns":0"#, Some(0))]
#[case::negative(r#""turns":-1"#, None)]
#[case::float(r#""turns":2.5"#, None)]
#[case::whole_float(r#""turns":2.0"#, None)]
#[case::string(r#""turns":"3""#, None)]
#[case::boolean(r#""turns":true"#, None)]
#[case::null(r#""turns":null"#, None)]
#[case::absent(r#""savings_estimated_turns":1"#, None)]
fn turns_fall_back_to_nothing_on_a_bad_value(#[case] extra: &str, #[case] expected: Option<u64>) {
    assert_eq!(parse(&with(extra)).unwrap().turns, expected);
}

#[rstest]
#[case::count(r#""savings_estimated_turns":4"#, Some(4))]
#[case::zero(r#""savings_estimated_turns":0"#, Some(0))]
#[case::absent(r#""turns":4"#, None)]
#[case::null(r#""savings_estimated_turns":null"#, None)]
#[case::negative_counts_as_zero(r#""savings_estimated_turns":-1"#, Some(0))]
#[case::float_counts_as_zero(r#""savings_estimated_turns":1.5"#, Some(0))]
#[case::string_counts_as_zero(r#""savings_estimated_turns":"4""#, Some(0))]
#[case::boolean_counts_as_zero(r#""savings_estimated_turns":true"#, Some(0))]
#[case::object_counts_as_zero(r#""savings_estimated_turns":{}"#, Some(0))]
fn estimated_turns_are_zero_when_present_but_invalid(
    #[case] extra: &str,
    #[case] expected: Option<u64>,
) {
    assert_eq!(
        parse(&with(extra)).unwrap().savings_estimated_turns,
        expected
    );
}

#[rstest]
#[case::amount(r#""savings_estimated_actual_spend":1.25"#, Some("1.25"))]
#[case::zero(r#""savings_estimated_actual_spend":0"#, Some("0"))]
#[case::negative(r#""savings_estimated_actual_spend":-0.01"#, None)]
#[case::string(r#""savings_estimated_actual_spend":"1""#, None)]
#[case::boolean(r#""savings_estimated_actual_spend":true"#, None)]
#[case::null(r#""savings_estimated_actual_spend":null"#, None)]
#[case::absent(r#""turns":1"#, None)]
fn estimated_actual_spend_is_a_non_negative_decimal_or_nothing(
    #[case] extra: &str,
    #[case] expected: Option<&str>,
) {
    let session = parse(&with(extra)).unwrap();

    assert_eq!(session.savings_estimated_actual_spend, expected.map(dec));
}

#[test]
fn a_session_serializes_under_the_names_litellms_cache_uses() {
    let session = parse(
        r#"{"router_name":"auto","last_model":"gpt-5","spend":1.50,
        "savings_estimated_baseline_spend":4,"baseline_model":"opus","turns":2}"#,
    )
    .unwrap();

    assert_eq!(
        serde_json::to_string(&session).unwrap(),
        r#"{"router_name":"auto","last_model":"gpt-5","spend":1.50,"baseline_spend":4,"baseline_model":"opus","turns":2,"savings_estimated_turns":null,"savings_estimated_actual_spend":null}"#
    );
}

#[test]
fn a_serialized_session_reads_back_unchanged() {
    let body = r#"{"router_name":"auto","last_model":"gpt-5","spend":1.50,
        "baseline_spend":4,"baseline_model":"opus","turns":2,"savings_estimated_turns":1,
        "savings_estimated_actual_spend":0.25}"#;
    let session = parse(body).unwrap();

    let stored = serde_json::to_vec(&session).unwrap();

    assert_eq!(Session::from_json(&stored), Some(session));
}
