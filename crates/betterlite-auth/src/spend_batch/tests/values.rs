use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_redis::cache_primitives::decode::CachedValue;
use serde_json::Number;

use crate::spend_batch::value::{CounterMapError, decimal_map};

use super::support::decimal;

const PRECISION_KEY: &str = "$serde_json::private::Number";

fn number(text: &str) -> CachedValue {
    CachedValue::Number(Number::from_str(text).expect("a fixture number parses"))
}

fn precision(text: &str) -> CachedValue {
    CachedValue::Object(BTreeMap::from([(
        PRECISION_KEY.to_owned(),
        CachedValue::Text(text.to_owned()),
    )]))
}

#[test]
fn decodes_every_counter_shape() {
    let values = BTreeMap::from([
        ("bool-false".to_owned(), Some(CachedValue::Boolean(false))),
        ("bool-true".to_owned(), Some(CachedValue::Boolean(true))),
        ("missing".to_owned(), Some(CachedValue::Null)),
        ("number".to_owned(), Some(number("2.5"))),
        ("precision".to_owned(), Some(precision("9.99"))),
        (
            "text".to_owned(),
            Some(CachedValue::Text("1.25".to_owned())),
        ),
    ]);
    assert_eq!(
        decimal_map(&values),
        Ok(BTreeMap::from([
            ("bool-false".to_owned(), Some(decimal("0"))),
            ("bool-true".to_owned(), Some(decimal("1"))),
            ("missing".to_owned(), None),
            ("number".to_owned(), Some(decimal("2.5"))),
            ("precision".to_owned(), Some(decimal("9.99"))),
            ("text".to_owned(), Some(decimal("1.25"))),
        ]))
    );
}

#[test]
fn array_value_rejects_the_whole_map() {
    let values = BTreeMap::from([
        ("good".to_owned(), Some(number("3"))),
        ("bad".to_owned(), Some(CachedValue::Array(Vec::new()))),
    ]);
    assert!(matches!(
        decimal_map(&values),
        Err(CounterMapError::WrongShape)
    ));
}

#[test]
fn precision_object_must_hold_exactly_the_number_text() {
    let extra = CachedValue::Object(BTreeMap::from([
        (PRECISION_KEY.to_owned(), CachedValue::Text("1".to_owned())),
        ("other".to_owned(), CachedValue::Text("2".to_owned())),
    ]));
    let foreign = CachedValue::Object(BTreeMap::from([(
        "other".to_owned(),
        CachedValue::Text("1".to_owned()),
    )]));
    let empty = CachedValue::Object(BTreeMap::new());
    let not_text = CachedValue::Object(BTreeMap::from([(
        PRECISION_KEY.to_owned(),
        CachedValue::Boolean(true),
    )]));
    for value in [extra, foreign, empty, not_text] {
        let values = BTreeMap::from([("bad".to_owned(), Some(value))]);
        assert!(matches!(
            decimal_map(&values),
            Err(CounterMapError::WrongShape)
        ));
    }
}

#[test]
fn non_decimal_text_names_the_value() {
    for text in ["not-a-number", "NaN", "1e40"] {
        let values = BTreeMap::from([("bad".to_owned(), Some(CachedValue::Text(text.to_owned())))]);
        match decimal_map(&values) {
            Err(CounterMapError::NotDecimal { text: named }) => assert_eq!(named, text),
            other => panic!("{text} must not decode: {other:?}"),
        }
    }
}

#[test]
fn scientific_text_decodes_like_the_budget_decoder() {
    let values = BTreeMap::from([(
        "amount".to_owned(),
        Some(CachedValue::Text("1.5e2".to_owned())),
    )]);
    assert_eq!(
        decimal_map(&values),
        Ok(BTreeMap::from([(
            "amount".to_owned(),
            Some(decimal("150"))
        )]))
    );
}
