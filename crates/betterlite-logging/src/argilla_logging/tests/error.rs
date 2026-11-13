use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::argilla_logging::extraction::{
    PayloadError, StandardLoggingPayload, chat_messages, response_value,
};

fn with_messages(messages: Json) -> StandardLoggingPayload {
    StandardLoggingPayload {
        messages: Some(messages),
        response: Some(Json::Text("done".to_owned())),
    }
}

fn with_response(response: Json) -> StandardLoggingPayload {
    StandardLoggingPayload {
        messages: Some(super::support::messages_json()),
        response: Some(response),
    }
}

#[test]
fn missing_messages_keep_litellms_message() {
    let error = chat_messages(&StandardLoggingPayload::default()).expect_err("no messages");
    assert_eq!(error.to_string(), "No chat messages found in payload.");
}

#[rstest]
#[case::empty_list(Json::List(Vec::new()))]
#[case::scalar(Json::Text("hi".to_owned()))]
#[case::number(Json::Number(serde_json::Number::from(7_u8)))]
#[case::list_of_scalars(Json::List(vec![Json::Text("hi".to_owned())]))]
fn invalid_messages_name_the_value(#[case] messages: Json) {
    let error = chat_messages(&with_messages(messages)).expect_err("invalid messages");
    assert!(matches!(error, PayloadError::InvalidChatMessages { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Invalid chat messages format:")
    );
}

#[test]
fn a_message_object_is_wrapped_in_a_list() {
    let message = Json::Object(IndexMap::from([(
        "role".to_owned(),
        Json::Text("user".to_owned()),
    )]));
    let messages = chat_messages(&with_messages(message.clone())).expect("one message");
    assert_eq!(messages, vec![message]);
}

#[test]
fn a_list_of_messages_is_returned_as_is() {
    let messages = super::support::messages_json();
    let returned = chat_messages(&with_messages(messages.clone())).expect("the list");
    assert_eq!(Json::List(returned), messages);
}

#[test]
fn missing_response_keeps_litellms_message() {
    let error = response_value(&StandardLoggingPayload::default()).expect_err("no response");
    assert_eq!(error.to_string(), "No response found in payload.");
}

#[test]
fn a_text_response_is_used_as_is() {
    let response =
        response_value(&with_response(Json::Text("done".to_owned()))).expect("a response");
    assert_eq!(response, Json::Text("done".to_owned()));
}

#[rstest]
#[case::first_choice(Json::Object(IndexMap::from([(
    "choices".to_owned(),
    Json::List(vec![Json::Object(IndexMap::from([(
        "message".to_owned(),
        Json::Object(IndexMap::from([(
            "content".to_owned(),
            Json::Text("picked".to_owned()),
        )])),
    )]))]),
)])), Json::Text("picked".to_owned()))]
#[case::no_choices_key(Json::Object(IndexMap::new()), Json::Text(String::new()))]
#[case::no_message_key(
    Json::Object(IndexMap::from([(
        "choices".to_owned(),
        Json::List(vec![Json::Object(IndexMap::new())]),
    )])),
    Json::Text(String::new())
)]
#[case::null_content(
    Json::Object(IndexMap::from([(
        "choices".to_owned(),
        Json::List(vec![Json::Object(IndexMap::from([(
            "message".to_owned(),
            Json::Object(IndexMap::from([("content".to_owned(), Json::Null)])),
        )]))]),
    )])),
    Json::Null
)]
fn reads_the_first_choice_content(#[case] response: Json, #[case] expected: Json) {
    assert_eq!(
        response_value(&with_response(response)).expect("a response"),
        expected
    );
}

#[rstest]
#[case::number(Json::Number(serde_json::Number::from(7_u8)))]
#[case::list(Json::List(Vec::new()))]
fn invalid_response_names_the_value(#[case] response: Json) {
    let error = response_value(&with_response(response)).expect_err("invalid response");
    assert!(matches!(error, PayloadError::InvalidResponse { .. }));
    assert!(error.to_string().starts_with("Invalid response format:"));
}

#[rstest]
#[case::empty_choices(Json::Object(IndexMap::from([(
    "choices".to_owned(),
    Json::List(Vec::new()),
)])))]
#[case::message_not_an_object(Json::Object(IndexMap::from([(
    "choices".to_owned(),
    Json::List(vec![Json::Object(IndexMap::from([(
        "message".to_owned(),
        Json::Null,
    )]))]),
)])))]
fn a_choice_without_a_message_is_a_shape_error(#[case] response: Json) {
    let error = response_value(&with_response(response)).expect_err("a shape error");
    assert!(matches!(error, PayloadError::ResponseShape));
}
