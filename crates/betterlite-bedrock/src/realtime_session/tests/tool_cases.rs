use super::support::{AUDIO_CONTENT, CONTENT, PROMPT, apply, prompt_start, session};
use crate::realtime_session::records::{MediaType, Tool, UpdateFrame};
use crate::realtime_session::tools::ToolDefinition;

pub(super) fn tool_configuration() {
    no_tools_for_empty_or_null();
    tools_from_function_entries();
    tools_keep_null_and_absent_fields();
    unsupported_tools_are_filtered();
    configuration_request_with_tools();
}

fn configuration_request_with_tools() {
    let state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let definitions: Vec<ToolDefinition> = serde_json::from_str(
        r#"[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}]"#,
    )
    .expect("a tool definition fixture");
    let configuration = state.configuration_request(Some(&definitions));
    let prompt_start = &configuration.prompt_start.event.prompt_start;
    assert_eq!(
        prompt_start.tool_use_output_configuration,
        Some(MediaType {
            media_type: "application/json".to_owned(),
        })
    );
    let tools = prompt_start
        .tool_configuration
        .as_ref()
        .expect("the configuration lists tools")
        .tools
        .as_slice();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_spec.name, Some("lookup".to_owned()));
    assert_eq!(tools[0].tool_spec.input_schema.json, r#"{"type":"object"}"#);
}

fn no_tools_for_empty_or_null() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let mut frames = apply(&mut state, r#"{"session":{"tools":[]}}"#, &[]);
    assert!(prompt_start(&frames).tool_configuration.is_none());
    assert!(
        prompt_start(&frames)
            .tool_use_output_configuration
            .is_none()
    );
    frames = apply(&mut state, r#"{"session":{"tools":null}}"#, &[]);
    assert!(prompt_start(&frames).tool_configuration.is_none());
    assert!(
        prompt_start(&frames)
            .tool_use_output_configuration
            .is_none()
    );
}

fn tools_from_function_entries() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"function","function":{"name":"get_weather","description":"Read the weather","parameters":{"type":"object","properties":{}}}}]}}"#,
        &[],
    );
    assert_eq!(
        prompt_start(&frames).tool_use_output_configuration,
        Some(MediaType {
            media_type: "application/json".to_owned(),
        })
    );
    let tools = configured_tools(&frames);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_spec.name, Some("get_weather".to_owned()));
    assert_eq!(
        tools[0].tool_spec.description,
        Some("Read the weather".to_owned())
    );
    assert_eq!(
        tools[0].tool_spec.input_schema.json,
        r#"{"type":"object","properties":{}}"#
    );
}

fn tools_keep_null_and_absent_fields() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let mut frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"function","function":{"name":"nulls","description":null,"parameters":null}}]}}"#,
        &[],
    );
    let nulls = configured_tools(&frames);
    assert_eq!(nulls.len(), 1);
    assert_eq!(nulls[0].tool_spec.name, Some("nulls".to_owned()));
    assert_eq!(nulls[0].tool_spec.description, None);
    assert_eq!(nulls[0].tool_spec.input_schema.json, "null");
    frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"function","name":"flattened","parameters":{"a":1}}]}}"#,
        &[],
    );
    let flattened = configured_tools(&frames);
    assert_eq!(flattened.len(), 1);
    assert_eq!(flattened[0].tool_spec.name, Some(String::new()));
    assert_eq!(flattened[0].tool_spec.description, Some(String::new()));
    assert_eq!(flattened[0].tool_spec.input_schema.json, "{}");
}

fn unsupported_tools_are_filtered() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let mut frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"computer_use_preview"},{"type":"function","function":{"name":"kept"}}]}}"#,
        &[],
    );
    let kept = configured_tools(&frames);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].tool_spec.name, Some("kept".to_owned()));
    assert_eq!(kept[0].tool_spec.description, Some(String::new()));
    assert_eq!(kept[0].tool_spec.input_schema.json, "{}");
    frames = apply(
        &mut state,
        r#"{"session":{"tools":[{"type":"computer_use_preview"}]}}"#,
        &[],
    );
    assert_eq!(
        prompt_start(&frames).tool_use_output_configuration,
        Some(MediaType {
            media_type: "application/json".to_owned(),
        })
    );
    assert!(configured_tools(&frames).is_empty());
}

fn configured_tools(frames: &[UpdateFrame]) -> &[Tool] {
    prompt_start(frames)
        .tool_configuration
        .as_ref()
        .expect("tools are configured")
        .tools
        .as_slice()
}
