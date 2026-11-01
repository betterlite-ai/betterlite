use super::*;

#[test]
fn message_fields_behavior_tests_behavior() {
    let text_run = vec![
        text_message("system", "existing"),
        text_message("user", "hi"),
    ];
    let merged_text = add_system_prompt_to_messages(&text_run, "  be nice  ", true);
    assert_eq!(merged_text.len(), 2);
    assert_eq!(
        merged_text[0].content,
        Some(Content::Text("be nice\n\nexisting".to_owned()))
    );

    let part_message = Message {
        role: "system".to_owned(),
        content: Some(Content::Parts(vec![text_part("existing part")])),
        name: None,
        provider_specific_fields: Object::new(),
    };
    let parts_run = vec![part_message, text_message("user", "hi")];
    let merged_parts = add_system_prompt_to_messages(&parts_run, "be nice", true);
    let Some(Content::Parts(parts)) = &merged_parts[0].content else {
        panic!("merged content must stay a parts list");
    };
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], text_part("be nice"));

    let empty_system = Message {
        role: "system".to_owned(),
        content: None,
        name: None,
        provider_specific_fields: Object::new(),
    };
    let absent_content_run = vec![empty_system, text_message("user", "hi")];
    let merged_absent = add_system_prompt_to_messages(&absent_content_run, "be nice", true);
    assert_eq!(
        merged_absent[0].content,
        Some(Content::Text("be nice\n\n".to_owned()))
    );

    assert_eq!(
        text_run[0].content,
        Some(Content::Text("existing".to_owned()))
    );
}
