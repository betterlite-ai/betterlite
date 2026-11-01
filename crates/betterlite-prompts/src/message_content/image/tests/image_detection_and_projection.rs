use super::*;

#[test]
fn message_content_behavior_tests_behavior() {
    let source = Object::from_iter([
        ("type".to_owned(), Json::Text("url".to_owned())),
        ("url".to_owned(), Json::Text("https://img".to_owned())),
    ]);
    let image = part(
        "image",
        Object::from_iter([("source".to_owned(), Json::Object(source))]),
    );
    let input = message(vec![image.clone()]);
    assert!(request_contains_image_content(&[input]));
    assert!(!message_has_audio_or_image(&message(vec![image.clone()])));
    assert_eq!(
        as_openai_image_part(&image).expect("image").kind,
        "image_url"
    );

    // Each disjunct of message_has_audio_or_image must independently report true.
    let image_url_part = part("image_url", Object::new());
    assert!(message_has_audio_or_image(&message(vec![image_url_part])));
    let audio_part = part("input_audio", Object::new());
    assert!(message_has_audio_or_image(&message(vec![audio_part])));
}
