#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::support::{index, list, number, object, text};
use crate::api::compression_interception::savings::{
    CompressionSavings, record_compression_savings, savings_from_counts,
};

fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("the fixture is finite"))
}

fn savings(before: u64, after: u64) -> CompressionSavings {
    CompressionSavings {
        tokens_before: before,
        tokens_after: after,
        tokens_saved: before.saturating_sub(after),
    }
}

#[test]
fn zero_counts_give_zero_saved() {
    assert_eq!(
        savings_from_counts(&number(0), &number(0)),
        Some(savings(0, 0))
    );
}

#[test]
fn positive_counts_give_the_difference() {
    assert_eq!(
        savings_from_counts(&number(100), &number(40)),
        Some(savings(100, 40))
    );
}

#[test]
fn a_bool_original_count_is_not_a_saving() {
    assert_eq!(savings_from_counts(&Json::Bool(true), &number(4)), None);
}

#[test]
fn a_bool_compressed_count_is_not_a_saving() {
    assert_eq!(savings_from_counts(&number(10), &Json::Bool(false)), None);
}

#[test]
fn a_float_original_count_is_not_a_saving() {
    assert_eq!(savings_from_counts(&float(10.5), &number(4)), None);
}

#[test]
fn a_float_compressed_count_is_not_a_saving() {
    assert_eq!(savings_from_counts(&number(10), &float(4.0)), None);
}

#[test]
fn a_negative_original_count_is_not_a_saving() {
    assert_eq!(savings_from_counts(&number(-10), &number(-12)), None);
}

#[test]
fn a_negative_compressed_count_is_not_a_saving() {
    assert_eq!(savings_from_counts(&number(10), &number(-4)), None);
}

#[test]
fn more_compressed_than_original_is_not_a_saving() {
    assert_eq!(savings_from_counts(&number(10), &number(40)), None);
}

#[test]
fn a_non_number_count_is_not_a_saving() {
    assert_eq!(savings_from_counts(&text("10"), &number(4)), None);
    assert_eq!(savings_from_counts(&Json::Null, &number(4)), None);
    assert_eq!(savings_from_counts(&list(Vec::new()), &number(4)), None);
    assert_eq!(savings_from_counts(&object(Vec::new()), &number(4)), None);
}

fn recorded(request: &IndexMap<String, Json>) -> Json {
    match request.get("litellm_metadata") {
        Some(Json::Object(metadata)) => metadata
            .get("compression_savings")
            .cloned()
            .expect("the savings are recorded"),
        _ => panic!("the metadata is an object"),
    }
}

#[test]
fn savings_attach_to_a_new_metadata_object() {
    let mut request = index(Vec::new());
    record_compression_savings(&mut request, &savings(100, 40));
    assert_eq!(
        recorded(&request),
        object(vec![
            ("tokens_before", number(100)),
            ("tokens_after", number(40)),
            ("tokens_saved", number(60)),
            ("source", text("compression_interception")),
        ])
    );
}

#[test]
fn savings_attach_in_place_to_an_existing_metadata_object() {
    let mut request = index(vec![(
        "litellm_metadata",
        object(vec![("team", text("t"))]),
    )]);
    record_compression_savings(&mut request, &savings(10, 10));
    let Some(Json::Object(metadata)) = request.get("litellm_metadata") else {
        panic!("the metadata stays an object");
    };
    assert_eq!(metadata.get("team"), Some(&text("t")));
    assert_eq!(
        metadata.get("compression_savings"),
        Some(&object(vec![
            ("tokens_before", number(10)),
            ("tokens_after", number(10)),
            ("tokens_saved", number(0)),
            ("source", text("compression_interception")),
        ]))
    );
}

#[test]
fn a_non_object_metadata_is_replaced() {
    let mut request = index(vec![("litellm_metadata", text("stale"))]);
    record_compression_savings(&mut request, &savings(5, 2));
    assert_eq!(
        recorded(&request),
        object(vec![
            ("tokens_before", number(5)),
            ("tokens_after", number(2)),
            ("tokens_saved", number(3)),
            ("source", text("compression_interception")),
        ])
    );
}
