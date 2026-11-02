use super::*;

#[test]
fn system_placement_behavior_tests_behavior() {
    let messages = vec![
        text_message("system", "a"),
        text_message("system", "b"),
        text_message("user", "hi"),
        text_message("assistant", "ok"),
    ];
    let (some_leading, some_rest) = split_leading_system_run(&messages);
    assert_eq!(some_leading, messages[..2]);
    assert_eq!(some_rest, messages[2..]);

    let no_leading = vec![text_message("user", "hi"), text_message("system", "a")];
    let (empty_leading, unchanged_rest) = split_leading_system_run(&no_leading);
    assert!(empty_leading.is_empty());
    assert_eq!(unchanged_rest, no_leading);

    let all_leading = vec![text_message("system", "a"), text_message("system", "b")];
    let (whole_leading, empty_rest) = split_leading_system_run(&all_leading);
    assert_eq!(whole_leading, all_leading);
    assert!(empty_rest.is_empty());
}
