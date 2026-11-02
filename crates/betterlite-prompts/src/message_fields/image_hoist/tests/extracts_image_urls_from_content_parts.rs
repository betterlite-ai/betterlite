use super::*;
use rstest::rstest;

#[rstest]
#[case::string_image_url_data_uri_keeps_only_the_base64_payload(
    vec![image_url_part_string("data:image/png;base64,iVBOR")],
    vec!["iVBOR".to_owned()]
)]
#[case::object_image_url_without_data_uri_is_kept_as_is(
    vec![image_url_part_object("https://example.com/cat.png")],
    vec!["https://example.com/cat.png".to_owned()]
)]
#[case::non_image_parts_contribute_nothing(
    vec![part("text", Object::from_iter([("text".to_owned(), Json::Text("hi".to_owned()))]))],
    Vec::<String>::new()
)]
#[case::image_url_object_missing_url_key_is_skipped(
    vec![part("image_url", Object::from_iter([("image_url".to_owned(), Json::Object(Object::new()))]))],
    Vec::<String>::new()
)]
#[case::image_url_object_with_non_string_url_is_skipped(
    vec![part(
        "image_url",
        Object::from_iter([(
            "image_url".to_owned(),
            Json::Object(Object::from_iter([(
                "url".to_owned(),
                Json::Number(serde_json::Number::from(1_i32)),
            )])),
        )]),
    )],
    Vec::<String>::new()
)]
fn message_fields_behavior_tests_behavior(
    #[case] parts: Vec<ContentPart>,
    #[case] expected: Vec<String>,
) {
    assert_eq!(
        extract_images_from_message(&message("user", parts)),
        expected
    );
}
