use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use super::support::{decimal, spend_json};
use crate::spend_reseed::rows::SpendAggregateRow;
use crate::spend_reseed::values::{
    ReseedCacheValue, SpendValueError, aggregate_amount, cache_amount, stored_amount,
};

fn sum_object(spend: &str) -> JsonData {
    JsonData::Object(BTreeMap::from([("spend".to_owned(), spend_json(spend))]))
}

fn mapping(sum: JsonData) -> SpendAggregateRow {
    SpendAggregateRow::Mapping(BTreeMap::from([("_sum".to_owned(), sum)]))
}

#[test]
fn mapping_and_attribute_sum_shapes() {
    assert_eq!(
        aggregate_amount(&mapping(sum_object("3.5"))),
        Ok(decimal("3.5"))
    );
    assert_eq!(
        aggregate_amount(&mapping(sum_object("\"2.25\""))),
        Ok(decimal("2.25"))
    );
    assert_eq!(
        aggregate_amount(&mapping(sum_object("false"))),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        aggregate_amount(&mapping(sum_object("[]"))),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        aggregate_amount(&mapping(sum_object("null"))),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        aggregate_amount(&SpendAggregateRow::Mapping(BTreeMap::new())),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        aggregate_amount(&mapping(JsonData::Null)),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        aggregate_amount(&mapping(spend_json("7"))),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        aggregate_amount(&mapping(sum_object("\"abc\""))),
        Err(SpendValueError::OutOfRange {
            text: "abc".to_owned()
        })
    );
    assert_eq!(
        aggregate_amount(&mapping(sum_object("[1]"))),
        Err(SpendValueError::NotNumeric { kind: "a list" })
    );
    assert_eq!(
        aggregate_amount(&mapping(sum_object("{\"a\": 1}"))),
        Err(SpendValueError::NotNumeric { kind: "an object" })
    );

    // a row object answers its _sum attribute, and a missing one reads as zero
    assert_eq!(
        aggregate_amount(&SpendAggregateRow::Attribute {
            sum: Some(sum_object("4.5"))
        }),
        Ok(decimal("4.5"))
    );
    assert_eq!(
        aggregate_amount(&SpendAggregateRow::Attribute { sum: None }),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        aggregate_amount(&SpendAggregateRow::Attribute {
            sum: Some(JsonData::Null)
        }),
        Ok(Decimal::ZERO)
    );
}

#[test]
fn stored_and_cache_amount_names() {
    assert_eq!(stored_amount(None), Ok(Decimal::ZERO));
    assert_eq!(stored_amount(Some(&JsonData::Null)), Ok(Decimal::ZERO));
    assert_eq!(stored_amount(Some(&JsonData::Bool(true))), Ok(Decimal::ONE));
    assert_eq!(
        stored_amount(Some(&JsonData::String(String::new()))),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        stored_amount(Some(&JsonData::String(" 2.5 ".to_owned()))),
        Ok(decimal("2.5"))
    );
    assert_eq!(
        stored_amount(Some(&JsonData::String("1e2".to_owned()))),
        Ok(decimal("100"))
    );
    assert_eq!(stored_amount(Some(&spend_json("[]"))), Ok(Decimal::ZERO));
    assert_eq!(stored_amount(Some(&spend_json("{}"))), Ok(Decimal::ZERO));
    assert_eq!(
        stored_amount(Some(&spend_json("\"nope\""))),
        Err(SpendValueError::OutOfRange {
            text: "nope".to_owned()
        })
    );

    assert_eq!(
        cache_amount(&ReseedCacheValue::Number(decimal("6.25"))),
        Ok(decimal("6.25"))
    );
    assert_eq!(
        cache_amount(&ReseedCacheValue::Boolean(false)),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        cache_amount(&ReseedCacheValue::Text(" 1.5 ".to_owned())),
        Ok(decimal("1.5"))
    );
    assert_eq!(
        cache_amount(&ReseedCacheValue::Text(String::new())),
        Err(SpendValueError::OutOfRange {
            text: String::new()
        })
    );
    assert_eq!(
        cache_amount(&ReseedCacheValue::Unreadable),
        Err(SpendValueError::NotNumeric {
            kind: "an unreadable cache value"
        })
    );
}
