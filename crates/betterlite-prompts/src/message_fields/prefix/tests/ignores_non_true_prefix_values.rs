use std::slice;

use super::*;
use rstest::rstest;

#[rstest]
#[case::prefix_false_is_left_untouched(Object::from_iter([("prefix".to_owned(), Json::Bool(false))]))]
#[case::prefix_explicit_null_is_left_untouched(Object::from_iter([("prefix".to_owned(), Json::Null)]))]
#[case::non_bool_prefix_is_left_untouched(Object::from_iter([("prefix".to_owned(), Json::Text("true".to_owned()))]))]
#[case::absent_prefix_field_is_left_untouched(Object::new())]
fn message_fields_behavior_tests_behavior(#[case] provider_specific_fields: Object) {
    let message = assistant_message("partial", provider_specific_fields);
    let converted = convert_prefix_messages(slice::from_ref(&message));
    assert_eq!(converted, vec![message]);
}
