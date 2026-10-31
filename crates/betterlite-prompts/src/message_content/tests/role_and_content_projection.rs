use super::*;

#[test]
fn message_content_behavior_tests_behavior() {
    let mut named = message("assistant", Some(Content::Text("a".to_owned())));
    named.name = Some("drop".to_owned());
    let messages = vec![
        named,
        message("system", Some(Content::Text("s".to_owned()))),
        message("tool", Some(Content::Text("t".to_owned()))),
    ];
    let allowed = vec!["user".to_owned()];
    assert_eq!(strip_name_from_messages(&messages, &allowed)[0].name, None);
    assert_eq!(get_str_from_messages(&messages), "ast");
}
