#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::finops::cloudzero::records::{CbfRecord, CbfValue};
use chrono::{TimeDelta, TimeZone as _, Utc};
use rust_decimal::Decimal;
use serde_json::Number;

use crate::finops::cloudzero::schedule::dry_run::preview_record;

fn moment() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 2, 3, 4, 5)
        .single()
        .expect("a valid instant")
}

#[test]
fn cloudzero_schedule() {
    let record = CbfRecord::from([
        ("none".to_owned(), CbfValue::None),
        ("text".to_owned(), CbfValue::Text("hello".to_owned())),
        ("integer".to_owned(), CbfValue::Integer(7)),
        ("money".to_owned(), CbfValue::Money(Decimal::new(123, 2))),
        ("moment".to_owned(), CbfValue::DateTime(moment())),
    ]);
    let preview = preview_record(&record).expect("a preview");
    assert_eq!(preview.len(), 5);
    assert_eq!(preview["none"], JsonData::Null);
    assert_eq!(preview["text"], JsonData::String("hello".to_owned()));
    assert_eq!(preview["integer"], JsonData::Number(Number::from(7_i64)));
    assert_eq!(
        preview["money"],
        JsonData::Number(serde_json::from_str("1.23").expect("a number"))
    );
    assert_eq!(
        preview["moment"],
        JsonData::String("2030-01-02T03:04:05".to_owned())
    );
}

mod fractional;
