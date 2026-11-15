use rust_decimal::Decimal;

use super::support::{
    AUDIO_CONTENT, CONTENT, INSTRUCTION, PROMPT, apply, instruction_content, output_audio,
    prompt_start, session, session_start,
};
use crate::realtime_session::records::{
    ContentRole, ContentType, ExactDecimal, InferenceConfiguration, Infinite, MaxResponseTokens,
    Setting, UpdateFrame,
};

pub(super) fn session_update_options() {
    updates_apply_by_key();
    updates_keep_absent_values();
    updates_accept_null_and_inf();
    instructions_add_content_frames();
}

pub(super) fn system_instructions() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let frames = apply(
        &mut state,
        r#"{"session":{"instructions":"You are a helpful assistant."}}"#,
        &[INSTRUCTION],
    );
    assert_eq!(frames.len(), 5);
    assert_instruction_frames(&frames, "You are a helpful assistant.");
    let mut empty = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let empty_frames = apply(&mut empty, r#"{"session":{"instructions":""}}"#, &[]);
    assert_eq!(empty_frames.len(), 2);
    let mut absent = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let absent_frames = apply(&mut absent, r#"{"session":{}}"#, &[]);
    assert_eq!(absent_frames.len(), 2);
}

fn updates_apply_by_key() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let mut frames = apply(&mut state, r#"{"type":"session.update"}"#, &[]);
    assert_eq!(frames.len(), 2);
    assert!(state.prompt_started);
    assert_eq!(
        inference(&frames).max_tokens,
        Setting(Some(MaxResponseTokens::Count(1024)))
    );
    assert_eq!(prompt_start(&frames).prompt_name, PROMPT);
    frames = apply(
        &mut state,
        r#"{"session":{"max_response_output_tokens":2048,"temperature":1.1,"voice":"amy","top_p":0.5}}"#,
        &[],
    );
    assert_eq!(
        inference(&frames).max_tokens,
        Setting(Some(MaxResponseTokens::Count(2048)))
    );
    assert_eq!(
        inference(&frames).temperature,
        Setting(Some(ExactDecimal(Decimal::new(11, 1))))
    );
    assert_eq!(inference(&frames).top_p, ExactDecimal(Decimal::new(9, 1)));
    assert_eq!(
        output_audio(&frames).voice_id,
        Setting(Some("amy".to_owned()))
    );
}

fn updates_keep_absent_values() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let applied = apply(
        &mut state,
        r#"{"session":{"max_response_output_tokens":2048,"temperature":1.1,"voice":"amy"}}"#,
        &[],
    );
    assert_eq!(applied.len(), 2);
    let frames = apply(&mut state, r#"{"session":{}}"#, &[]);
    assert_eq!(
        inference(&frames).max_tokens,
        Setting(Some(MaxResponseTokens::Count(2048)))
    );
    assert_eq!(
        inference(&frames).temperature,
        Setting(Some(ExactDecimal(Decimal::new(11, 1))))
    );
    assert_eq!(
        output_audio(&frames).voice_id,
        Setting(Some("amy".to_owned()))
    );
}

fn updates_accept_null_and_inf() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let mut frames = apply(
        &mut state,
        r#"{"session":{"max_response_output_tokens":null,"temperature":null,"voice":""}}"#,
        &[],
    );
    assert_eq!(inference(&frames).max_tokens, Setting(None));
    assert_eq!(inference(&frames).temperature, Setting(None));
    assert_eq!(output_audio(&frames).voice_id, Setting(Some(String::new())));
    frames = apply(
        &mut state,
        r#"{"session":{"max_response_output_tokens":"inf"}}"#,
        &[],
    );
    assert_eq!(
        inference(&frames).max_tokens,
        Setting(Some(MaxResponseTokens::Infinite(Infinite::Inf)))
    );
    frames = apply(
        &mut state,
        r#"{"session":{"unknown_field":1,"unknown_nested":{"a":1},"tools":[],"instructions":""}}"#,
        &[],
    );
    assert_eq!(
        inference(&frames).max_tokens,
        Setting(Some(MaxResponseTokens::Infinite(Infinite::Inf)))
    );
    assert!(prompt_start(&frames).tool_configuration.is_none());
    assert!(
        prompt_start(&frames)
            .tool_use_output_configuration
            .is_none()
    );
    assert_eq!(frames.len(), 2);
}

fn instructions_add_content_frames() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let prompt_name = state.names.prompt.clone();
    let frames = apply(
        &mut state,
        r#"{"session":{"instructions":"be helpful"}}"#,
        &[INSTRUCTION],
    );
    assert_eq!(frames.len(), 5);
    assert_eq!(state.names.prompt, prompt_name);
    assert_eq!(state.names.content, CONTENT);
    assert_eq!(state.names.audio_content, AUDIO_CONTENT);
    assert!(state.prompt_started);
    assert_instruction_frames(&frames, "be helpful");
    let empty = apply(&mut state, r#"{"session":{"instructions":""}}"#, &[]);
    assert_eq!(empty.len(), 2);
    let null = apply(&mut state, r#"{"session":{"instructions":null}}"#, &[]);
    assert_eq!(null.len(), 2);
    let absent = apply(&mut state, r#"{"session":{}}"#, &[]);
    assert_eq!(absent.len(), 2);
}

fn inference(frames: &[UpdateFrame]) -> &InferenceConfiguration {
    &session_start(frames).inference_configuration
}

fn assert_instruction_frames(frames: &[UpdateFrame], content: &str) {
    let instruction = instruction_content(frames);
    assert_eq!(instruction.content_start.prompt_name, PROMPT);
    assert_eq!(instruction.content_start.content_name, INSTRUCTION);
    assert_eq!(instruction.content_start.content_type, ContentType::Text);
    assert_eq!(instruction.content_start.role, ContentRole::System);
    assert!(!instruction.content_start.interactive);
    assert_eq!(
        instruction
            .content_start
            .text_input_configuration
            .media_type,
        "text/plain"
    );
    assert_eq!(instruction.text_input, (PROMPT, INSTRUCTION, content));
    assert_eq!(instruction.content_end, (PROMPT, INSTRUCTION));
}
