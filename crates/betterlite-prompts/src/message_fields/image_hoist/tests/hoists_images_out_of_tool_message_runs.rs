use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    asserts_no_images_pass_through_unchanged();
    asserts_image_run_hoists_into_an_inserted_user_message();
    asserts_an_image_only_tool_run_gets_placeholder_text();
}

fn asserts_no_images_pass_through_unchanged() {
    let no_images = vec![
        message("user", vec![part("text", Object::new())]),
        message("tool", vec![part("text", Object::new())]),
    ];
    assert_eq!(hoist_tool_message_images(&no_images), no_images);
}

fn asserts_image_run_hoists_into_an_inserted_user_message() {
    let tool_with_image = message(
        "tool",
        vec![
            part(
                "text",
                Object::from_iter([("text".to_owned(), Json::Text("ok".to_owned()))]),
            ),
            image_url_part_string("data:image/png;base64,abc"),
        ],
    );
    let messages = vec![
        message("assistant", vec![]),
        tool_with_image,
        message("user", vec![part("text", Object::new())]),
    ];

    let hoisted = hoist_tool_message_images(&messages);

    assert_eq!(hoisted.len(), 4);
    assert_eq!(hoisted[0].role, "assistant");
    assert_eq!(hoisted[1].role, "tool");
    assert_eq!(
        hoisted[1].content,
        Some(Content::Parts(vec![part(
            "text",
            Object::from_iter([("text".to_owned(), Json::Text("ok".to_owned()))])
        )]))
    );
    assert_eq!(hoisted[2].role, "user");
    let Some(Content::Parts(inserted_parts)) = &hoisted[2].content else {
        panic!("the inserted message must carry content parts");
    };
    assert_eq!(inserted_parts[0].kind, "text");
    assert_eq!(inserted_parts[1].kind, "image_url");
    assert_eq!(hoisted[3].role, "user");
}

fn asserts_an_image_only_tool_run_gets_placeholder_text() {
    let only_image_tool = vec![message(
        "tool",
        vec![image_url_part_string("data:image/png;base64,abc")],
    )];

    let placeholder_result = hoist_tool_message_images(&only_image_tool);

    assert_eq!(
        placeholder_result[0].content,
        Some(Content::Text(TOOL_RESULT_IMAGE_PLACEHOLDER.to_owned()))
    );
    assert_eq!(
        only_image_tool[0].role, "tool",
        "the source messages must not be mutated"
    );
}
