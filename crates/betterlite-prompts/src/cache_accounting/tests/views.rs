use rstest::rstest;

use crate::cache_accounting::json_views::object_list;
use crate::cache_accounting::tests::support::json;

#[rstest]
#[case::list("[]", true)]
#[case::list_of_objects(r#"[{"a": 1}, 2]"#, true)]
#[case::object("{}", false)]
#[case::null("null", false)]
#[case::text(r#""x""#, false)]
#[case::number("1", false)]
#[case::boolean("true", false)]
fn object_list_accepts_only_lists(#[case] value: &str, #[case] expected: bool) {
    assert_eq!(object_list(&json(value)).is_some(), expected);
}

#[test]
fn object_list_keeps_the_original_structure() {
    let value = json(r#"[{"cache_control": {}}, "text"]"#);
    let items = object_list(&value).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        serde_json::to_string(&items[0]).unwrap(),
        r#"{"cache_control":{}}"#
    );
    assert_eq!(serde_json::to_string(&items[1]).unwrap(), r#""text""#);
}
