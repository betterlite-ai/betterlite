use super::support::{AUDIO_CONTENT, CONTENT, PROMPT, apply, prompt_start, session};
use crate::realtime_session::records::MediaType;

pub(super) fn tool_configuration() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let frames = apply(&mut state, r#"{"session":{"tools":[]}}"#, &[]);
    assert!(prompt_start(&frames).tool_configuration.is_none());
    assert!(
        prompt_start(&frames)
            .tool_use_output_configuration
            .is_none()
    );

    let frames = apply(&mut state, r#"{"session":{"tools":null}}"#, &[]);
    assert!(prompt_start(&frames).tool_configuration.is_none());
    assert!(
        prompt_start(&frames)
            .tool_use_output_configuration
            .is_none()
    );

    let frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"function","function":{"name":"get_weather","description":"Read the weather","parameters":{"type":"object","properties":{}}}}]}}"#,
        &[],
    );
    let prompt = prompt_start(&frames);
    assert_eq!(
        prompt.tool_use_output_configuration,
        Some(MediaType {
            media_type: "application/json".to_owned(),
        })
    );
    let tools = &prompt
        .tool_configuration
        .as_ref()
        .expect("tools are configured")
        .tools;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_spec.name, "get_weather");
    assert_eq!(tools[0].tool_spec.description, "Read the weather");
    assert_eq!(
        tools[0].tool_spec.input_schema.json,
        r#"{"type":"object","properties":{}}"#
    );

    let frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"function","function":{"name":"nulls","description":null,"parameters":null}}]}}"#,
        &[],
    );
    let tools = &prompt_start(&frames)
        .tool_configuration
        .as_ref()
        .expect("tools are configured")
        .tools;
    assert_eq!(tools[0].tool_spec.name, "nulls");
    assert_eq!(tools[0].tool_spec.description, "");
    assert_eq!(tools[0].tool_spec.input_schema.json, "null");

    let frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"function","name":"flattened","parameters":{"a":1}}]}}"#,
        &[],
    );
    let tools = &prompt_start(&frames)
        .tool_configuration
        .as_ref()
        .expect("tools are configured")
        .tools;
    assert_eq!(tools[0].tool_spec.name, "");
    assert_eq!(tools[0].tool_spec.description, "");
    assert_eq!(tools[0].tool_spec.input_schema.json, "{}");

    let frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"computer_use_preview"},{"type":"function","function":{"name":"kept"}}]}}"#,
        &[],
    );
    let tools = &prompt_start(&frames)
        .tool_configuration
        .as_ref()
        .expect("a truthy list configures tools")
        .tools;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_spec.name, "kept");

    let frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"computer_use_preview"}]}}"#,
        &[],
    );
    let prompt = prompt_start(&frames);
    assert_eq!(
        prompt.tool_use_output_configuration,
        Some(MediaType {
            media_type: "application/json".to_owned(),
        })
    );
    assert!(
        prompt
            .tool_configuration
            .as_ref()
            .expect("an empty list is configured")
            .tools
            .is_empty()
    );
}
