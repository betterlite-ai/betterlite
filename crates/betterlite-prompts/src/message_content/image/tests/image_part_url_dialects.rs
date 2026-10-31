use super::*;
use rstest::rstest;

#[rstest]
#[case::image_url_string(
    part(
        "image_url",
        Object::from_iter([("image_url".to_owned(), Json::Text("https://a".to_owned()))]),
    ),
    Some("https://a")
)]
#[case::image_url_object(
    part(
        "image_url",
        Object::from_iter([(
            "image_url".to_owned(),
            Json::Object(Object::from_iter([(
                "url".to_owned(),
                Json::Text("https://b".to_owned()),
            )])),
        )]),
    ),
    Some("https://b")
)]
#[case::input_image(
    part(
        "input_image",
        Object::from_iter([("image_url".to_owned(), Json::Text("https://c".to_owned()))]),
    ),
    Some("https://c")
)]
#[case::unknown_kind(part("input_audio", Object::new()), None)]
fn message_content_behavior_tests_behavior(
    #[case] input: ContentPart,
    #[case] expected: Option<&str>,
) {
    let converted = as_openai_image_part(&input).map(|part| {
        let Some(Json::Object(image_url)) = part.fields.get("image_url") else {
            panic!("image_url field missing")
        };
        image_url
            .get("url")
            .and_then(Json::as_str)
            .expect("url")
            .to_owned()
    });
    assert_eq!(converted, expected.map(str::to_owned));
}
