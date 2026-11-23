#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{reservation_from_metadata, stamp_callback_bound};

fn mapping(encoded: &str) -> IndexMap<String, Json> {
    serde_json::from_str(encoded).expect("a mapping")
}

fn value(encoded: &str) -> Json {
    serde_json::from_str(encoded).expect("a value")
}

fn stamped(litellm_params: &str, callback_bound: bool) -> IndexMap<String, Json> {
    let mut params = mapping(litellm_params);
    stamp_callback_bound(&mut params, callback_bound);
    params
}

fn encoded(mapping: &IndexMap<String, Json>) -> String {
    serde_json::to_string(mapping).expect("a mapping serializes")
}

#[rstest]
#[case::direct_stamp_wins(
    r#"{"user_api_key_budget_reservation": {"id": "direct"}, "user_api_key_auth": {"budget_reservation": {"id": "auth"}}}"#,
    r#"{"id": "direct"}"#
)]
#[case::empty_direct_stamp_still_wins(
    r#"{"user_api_key_budget_reservation": {}, "user_api_key_auth": {"budget_reservation": {"id": "auth"}}}"#,
    "{}"
)]
#[case::auth_record_alone(
    r#"{"user_api_key_auth": {"budget_reservation": {"id": "auth"}}, "team": "t1"}"#,
    r#"{"id": "auth"}"#
)]
fn lookup_prefers_an_object_direct_stamp(#[case] metadata: &str, #[case] expected: &str) {
    let recorded = mapping(metadata);
    let found = reservation_from_metadata(&recorded);
    assert_eq!(found, Some(&mapping(expected)));
}

#[rstest]
#[case::null_stamp("null")]
#[case::list_stamp(r#"["res"]"#)]
#[case::text_stamp(r#""res""#)]
#[case::number_stamp("7")]
#[case::bool_stamp("true")]
fn lookup_non_object_direct_stamp_falls_back_to_the_auth_record(#[case] stamp: &str) {
    let mut metadata = mapping(r#"{"user_api_key_auth": {"budget_reservation": {"id": "auth"}}}"#);
    metadata.insert("user_api_key_budget_reservation".to_owned(), value(stamp));
    assert_eq!(
        reservation_from_metadata(&metadata),
        Some(&mapping(r#"{"id": "auth"}"#))
    );
}

#[rstest]
#[case::list_auth(r#"["user"]"#)]
#[case::text_auth(r#""user""#)]
#[case::null_auth("null")]
#[case::number_auth("7")]
fn lookup_auth_record_that_is_not_an_object_is_absent(#[case] auth: &str) {
    let mut metadata = mapping(r#"{"user_api_key_budget_reservation": null}"#);
    metadata.insert("user_api_key_auth".to_owned(), value(auth));
    assert_eq!(reservation_from_metadata(&metadata), None);
}

#[rstest]
#[case::auth_record_without_a_reservation("{}")]
#[case::auth_record_with_another_member(r#"{"team": "t1"}"#)]
#[case::reservation_is_null(r#"{"budget_reservation": null}"#)]
#[case::reservation_is_a_list(r#"{"budget_reservation": [1]}"#)]
#[case::reservation_is_text(r#"{"budget_reservation": "res"}"#)]
fn lookup_auth_record_without_an_object_reservation_is_absent(#[case] auth: &str) {
    let mut metadata = mapping("{}");
    metadata.insert("user_api_key_auth".to_owned(), value(auth));
    assert_eq!(reservation_from_metadata(&metadata), None);
}

#[test]
fn lookup_nothing_recorded_is_absent() {
    assert_eq!(reservation_from_metadata(&mapping("{}")), None);
    assert_eq!(
        reservation_from_metadata(&mapping(r#"{"team": "t1", "user_api_key_auth": null}"#)),
        None
    );
}

#[rstest]
#[case::both_buckets(
    r#"{"metadata": {"user_api_key_budget_reservation": {"id": "res-1"}}, "litellm_metadata": {"user_api_key_budget_reservation": {"id": "res-2"}}, "model": "gpt-4"}"#,
    r#"{"metadata":{"user_api_key_budget_reservation":{"id":"res-1","callback_bound":true}},"litellm_metadata":{"user_api_key_budget_reservation":{"id":"res-2","callback_bound":true}},"model":"gpt-4"}"#
)]
#[case::only_litellm_metadata(
    r#"{"metadata": {"team": "t1"}, "litellm_metadata": {"user_api_key_budget_reservation": {"id": "res-2"}}}"#,
    r#"{"metadata":{"team":"t1"},"litellm_metadata":{"user_api_key_budget_reservation":{"id":"res-2","callback_bound":true}}}"#
)]
#[case::reservation_on_the_auth_record(
    r#"{"metadata": {"user_api_key_auth": {"budget_reservation": {"id": "res-3"}}}}"#,
    r#"{"metadata":{"user_api_key_auth":{"budget_reservation":{"id":"res-3","callback_bound":true}}}}"#
)]
#[case::empty_reservation_is_stamped(
    r#"{"litellm_metadata": {"user_api_key_budget_reservation": {}}}"#,
    r#"{"litellm_metadata":{"user_api_key_budget_reservation":{"callback_bound":true}}}"#
)]
fn stamp_sets_callback_bound_on_each_bucket(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(encoded(&stamped(input, true)), expected);
}

#[rstest]
#[case::list_bucket(
    r#"{"metadata": [1], "litellm_metadata": {"user_api_key_budget_reservation": {"id": "res"}}}"#,
    r#"{"metadata":[1],"litellm_metadata":{"user_api_key_budget_reservation":{"id":"res","callback_bound":true}}}"#
)]
#[case::text_bucket(
    r#"{"metadata": "stamped", "litellm_metadata": {"user_api_key_budget_reservation": {"id": "res"}}}"#,
    r#"{"metadata":"stamped","litellm_metadata":{"user_api_key_budget_reservation":{"id":"res","callback_bound":true}}}"#
)]
#[case::null_bucket(
    r#"{"metadata": null, "litellm_metadata": {"user_api_key_budget_reservation": {"id": "res"}}}"#,
    r#"{"metadata":null,"litellm_metadata":{"user_api_key_budget_reservation":{"id":"res","callback_bound":true}}}"#
)]
#[case::no_bucket_is_created("{}", "{}")]
#[case::bucket_without_a_reservation_stays(
    r#"{"metadata":{"team":"t1"},"litellm_metadata":{"user_api_key_auth":{"team":"t2"}}}"#,
    r#"{"metadata":{"team":"t1"},"litellm_metadata":{"user_api_key_auth":{"team":"t2"}}}"#
)]
#[case::non_object_reservation_stays(
    r#"{"metadata":{"user_api_key_budget_reservation":"res"},"litellm_metadata":{"user_api_key_auth":null}}"#,
    r#"{"metadata":{"user_api_key_budget_reservation":"res"},"litellm_metadata":{"user_api_key_auth":null}}"#
)]
fn stamp_skips_a_bucket_it_cannot_stamp(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(encoded(&stamped(input, true)), expected);
}

#[rstest]
#[case::existing_member_stays_in_place(
    r#"{"metadata": {"user_api_key_budget_reservation": {"callback_bound": false, "id": "res", "cost": 1}, "team": "t1"}}"#,
    r#"{"metadata":{"user_api_key_budget_reservation":{"callback_bound":true,"id":"res","cost":1},"team":"t1"}}"#
)]
#[case::missing_member_is_appended(
    r#"{"metadata": {"user_api_key_budget_reservation": {"id": "res"}}}"#,
    r#"{"metadata":{"user_api_key_budget_reservation":{"id":"res","callback_bound":true}}}"#
)]
fn stamp_keeps_the_callback_bound_member_in_place(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(encoded(&stamped(input, true)), expected);
}

#[test]
fn stamp_unbind_after_bind_leaves_false() {
    let input = r#"{"metadata": {"user_api_key_budget_reservation": {"id": "res"}}, "litellm_metadata": {"user_api_key_auth": {"budget_reservation": {"id": "res"}}}}"#;
    let mut params = stamped(input, true);
    stamp_callback_bound(&mut params, false);
    assert_eq!(
        encoded(&params),
        r#"{"metadata":{"user_api_key_budget_reservation":{"id":"res","callback_bound":false}},"litellm_metadata":{"user_api_key_auth":{"budget_reservation":{"id":"res","callback_bound":false}}}}"#
    );
}
