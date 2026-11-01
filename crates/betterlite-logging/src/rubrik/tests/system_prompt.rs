use betterlite_logging::rubrik::system_prompt::prepend_system_prompt;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

fn scaffold(content: &Json) -> Json {
    Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("system".to_owned())),
        ("content".to_owned(), content.clone()),
    ]))
}

#[test]
fn is_a_no_op_when_the_system_prompt_is_absent() {
    let messages = Json::List(vec![Json::Text("hi".to_owned())]);
    assert_eq!(prepend_system_prompt(Some(&messages), None), None);
}

#[test]
fn is_a_no_op_when_the_system_prompt_is_an_empty_string() {
    let messages = Json::List(vec![Json::Text("hi".to_owned())]);
    let system = Json::Text(String::new());
    assert_eq!(prepend_system_prompt(Some(&messages), Some(&system)), None);
}

#[test]
fn prepends_a_system_scaffold_onto_a_list_of_messages() {
    let original = vec![Json::Text("hi".to_owned())];
    let messages = Json::List(original.clone());
    let system = Json::Text("be nice".to_owned());
    let combined = prepend_system_prompt(Some(&messages), Some(&system)).expect("prepends");
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[0], scaffold(&system));
    assert_eq!(combined[1], original[0]);
}

#[test]
fn wraps_a_dict_shaped_messages_value_with_the_scaffold() {
    let messages = Json::Object(IndexMap::from([(
        "role".to_owned(),
        Json::Text("user".to_owned()),
    )]));
    let system = Json::Text("be nice".to_owned());
    let combined = prepend_system_prompt(Some(&messages), Some(&system)).expect("prepends");
    assert_eq!(combined, vec![scaffold(&system), messages]);
}

#[test]
fn wraps_a_string_shaped_messages_value_with_the_scaffold() {
    let messages = Json::Text("raw prompt".to_owned());
    let system = Json::Text("be nice".to_owned());
    let combined = prepend_system_prompt(Some(&messages), Some(&system)).expect("prepends");
    assert_eq!(combined, vec![scaffold(&system), messages]);
}

#[test]
fn leaves_the_payload_alone_for_an_unsupported_messages_shape() {
    let messages = Json::Number(serde_json::Number::from(1_i32));
    let system = Json::Text("be nice".to_owned());
    assert_eq!(prepend_system_prompt(Some(&messages), Some(&system)), None);
}

#[test]
fn leaves_the_payload_alone_when_messages_are_entirely_absent() {
    let system = Json::Text("be nice".to_owned());
    assert_eq!(prepend_system_prompt(None, Some(&system)), None);
}
