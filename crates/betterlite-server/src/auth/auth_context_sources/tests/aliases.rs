use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::auth::auth_context_sources::alias::resolve_model_group_alias;

#[rstest]
#[case::absent(None, None)]
#[case::null(Some("null"), None)]
#[case::wrong_map(Some("[]"), None)]
#[case::missing(Some(r#"{"other":"group"}"#), None)]
#[case::plain(Some(r#"{"alias":"group"}"#), Some("group"))]
#[case::item(Some(r#"{"alias":{"model":"group","hidden":true}}"#), Some("group"))]
#[case::self_target(Some(r#"{"alias":"alias"}"#), Some("alias"))]
#[case::empty(Some(r#"{"alias":""}"#), None)]
#[case::null_target(Some(r#"{"alias":null}"#), None)]
#[case::boolean(Some(r#"{"alias":true}"#), None)]
#[case::number(Some(r#"{"alias":7}"#), None)]
#[case::list(Some(r#"{"alias":[]}"#), None)]
#[case::missing_model(Some(r#"{"alias":{}}"#), None)]
#[case::empty_model(Some(r#"{"alias":{"model":""}}"#), None)]
#[case::wrong_model(Some(r#"{"alias":{"model":7}}"#), None)]
fn alias_entry_shapes(#[case] aliases: Option<&str>, #[case] expected: Option<&str>) {
    let parsed = aliases.map(|text| serde_json::from_str::<Json>(text).unwrap());
    assert_eq!(
        resolve_model_group_alias(parsed.as_ref(), "alias").as_deref(),
        expected
    );
}
